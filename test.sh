#!/bin/bash

# ============================================
# TejX Unified Test Runner (High-Performance Parallel)
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/build/tests"

# Compiler mode configuration:
# - "build" (default): runs ./build.sh and uses local target/release/tejxc
# - "installed": uses installed tejxc from PATH or ~/.tejx/bin/tejxc
# - "custom": uses path supplied via --compiler <path> or TEJXC env var
COMPILER_MODE="build"
TEJXC_BIN="${TEJXC:-}"
DO_BUILD=true
CUSTOM_STDLIB=""
CUSTOM_RUNTIME=""
STDLIB_PATH=""
RUNTIME_PATH=""
COMPILER_ARGS=()

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Parallel configuration
MAX_JOBS=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)

# Configuration
RUN_POSITIVE=false
RUN_NEGATIVE=false
RUN_PROBLEMS=false
FILTER=""
SPECIFIC_PATHS=()

# Timeout helper (macOS compatible)
run_with_timeout() {
    local timeout=$1
    shift
    "$@" &
    local child_pid=$!
    (
        sleep "$timeout"
        kill -SIGHUP "$child_pid" 2>/dev/null
        sleep 1
        kill -9 "$child_pid" 2>/dev/null
    ) >/dev/null 2>&1 &
    local watcher_pid=$!
    disown "$watcher_pid" 2>/dev/null
    wait "$child_pid" 2>/dev/null
    local exit_code=$?
    kill -9 "$watcher_pid" 2>/dev/null
    return "$exit_code"
}

output_has_assertion_failure() {
    local out_file=$1
    grep -Eq '❌ (FAIL|ASSERT FAILED)|AssertionFailed:' "$out_file"
}

runtime_timeout_for() {
    local file=$1
    local rel_file="$file"
    [[ "$rel_file" == "$SCRIPT_DIR/"* ]] && rel_file="${rel_file#$SCRIPT_DIR/}"
    [[ "$rel_file" == "./"* ]] && rel_file="${rel_file#./}"
    case "$rel_file" in
        tests/positive/std/net.tx|tests/positive/std/promise_parallel.tx) echo 25 ;;
        tests/positive/std/thread.tx|tests/problems/producer_consumer.tx) echo 15 ;;
        tests/positive/vthread_deep_stack.tx|tests/positive/vthread_stress.tx|tests/positive/vthread_mem_stress.tx) echo 30 ;;
        tests/problems/benchmark.tx) echo 120 ;;
        *) echo 10 ;;
    esac
}

print_header() {
    echo -e "${CYAN}============================================${NC}"
    echo -e "${CYAN}   TejX Test Runner: $1 (${MAX_JOBS} parallel workers)${NC}"
    echo -e "${CYAN}   Compiler: ${TEJXC_BIN} (${COMPILER_MODE})${NC}"
    echo -e "${CYAN}============================================${NC}"
}

run_test_file() {
    local file=$1
    local type=$2 # positive, negative, problem
    local id=$3
    
    local norm_path="$file"
    [[ "$norm_path" == "$SCRIPT_DIR/"* ]] && norm_path="${norm_path#$SCRIPT_DIR/}"
    [[ "$norm_path" == "./"* ]] && norm_path="${norm_path#./}"
    local rel_path="$norm_path"
    local filename=$(basename "${file%.*}")
    
    local test_log="$RESULTS_DIR/log_${id}.txt"
    local out_file="$RESULTS_DIR/out_${id}.txt"
    local compile_out="$RESULTS_DIR/cout_${id}.txt"
    local err_detail="$RESULTS_DIR/${id}.detail"
    local compile_timeout=60
    
    local binary="${file%.*}"
    local ll_file="${file%.*}.ll"
    rm -f "$binary" "$ll_file" "${file%.*}.o" "${file%.*}.s"
    
    local test_passed=false
    local err_reason=""
    
    (
        echo "----------------------------------------"
        if [ "$type" == "negative" ]; then
            echo -e "${YELLOW}Testing: $rel_path${NC}"
            
            # Read expected failure type and description
            local expected_type="COMPILE_ERROR"
            grep -q "EXPECTED: RUNTIME_ERROR" "$file" && expected_type="RUNTIME_ERROR"
            local description=$(grep "Description:" "$file" | cut -d ':' -f 2- | xargs)
            
            echo -e "  Description: $description"
            echo -e "  Expected:    ${CYAN}$expected_type${NC}"
            
            run_with_timeout 20 "$TEJXC_BIN" "${COMPILER_ARGS[@]}" "$file" > "$compile_out" 2>&1
            local compile_exit=$?
            
            local actual=""
            if [ $compile_exit -eq 129 ] || [ $compile_exit -eq 143 ] || [ $compile_exit -eq 137 ]; then
                actual="COMPILE_TIMEOUT (HANG)"
            elif [ $compile_exit -ne 0 ]; then
                actual="COMPILE_ERROR"
            else
                actual="COMPILE_SUCCESS"
            fi

            if [ "$expected_type" == "COMPILE_ERROR" ]; then
                if [ "$actual" == "COMPILE_ERROR" ] || [ "$actual" == "COMPILE_TIMEOUT (HANG)" ]; then
                    echo -e "  Actual:      ${GREEN}$actual${NC}"
                    echo -e "  Error Log:   $(grep -v "Terminated: 15" "$compile_out" | grep -v "sleep" | head -n 2 | tr '\n' ' ')"
                    echo -e "  Result:      ${GREEN}✅ PASS${NC}"
                    test_passed=true
                else
                    echo -e "  Actual:      ${RED}$actual${NC}"
                    echo -e "  Result:      ${RED}❌ FAIL (Expected compilation to fail)${NC}"
                    test_passed=false
                    err_reason="Expected COMPILE_ERROR, got $actual"
                    echo "Expected compile error, but compilation succeeded unexpectedly." > "$err_detail"
                fi
            else # EXPECTED: RUNTIME_ERROR
                if [ "$actual" == "COMPILE_SUCCESS" ]; then
                    if [ -f "$binary" ]; then
                        run_with_timeout 5 "$binary" > "$out_file" 2>&1
                        local run_exit=$?
                        
                        local actual_runtime=""
                        if [ $run_exit -eq 129 ] || [ $run_exit -eq 143 ] || [ $run_exit -eq 137 ]; then
                            actual_runtime="RUNTIME_TIMEOUT (HANG)"
                        elif [ $run_exit -ne 0 ]; then
                            actual_runtime="RUNTIME_ERROR (CRASH)"
                        else
                            actual_runtime="RUNTIME_SUCCESS"
                        fi
                        
                        echo -e "  Actual:      ${YELLOW}$actual_runtime${NC}"
                        if [[ "$actual_runtime" == *"ERROR"* ]] || [[ "$actual_runtime" == *"TIMEOUT"* ]]; then
                            echo -e "  Runtime Log: $(head -n 2 "$out_file" | tr '\n' ' ')"
                            echo -e "  Result:      ${GREEN}✅ PASS${NC}"
                            test_passed=true
                        else
                            echo -e "  Result:      ${RED}❌ FAIL (Expected runtime error, but ran successfully)${NC}"
                            test_passed=false
                            err_reason="Expected RUNTIME_ERROR, got RUNTIME_SUCCESS"
                            echo "Expected runtime crash/error, but program exited 0." > "$err_detail"
                        fi
                    else
                        echo -e "  Actual:      ${RED}NO_BINARY_GENERATED${NC}"
                        echo -e "  Result:      ${RED}❌ FAIL${NC}"
                        test_passed=false
                        err_reason="Expected RUNTIME_ERROR, but no binary was generated"
                        echo "No binary was generated." > "$err_detail"
                    fi
                else
                    echo -e "  Actual:      ${RED}$actual${NC} (Caught too early?)"
                    echo -e "  Result:      ${RED}❌ FAIL${NC}"
                    test_passed=false
                    err_reason="Expected RUNTIME_ERROR, but failed at compile time"
                    cat "$compile_out" > "$err_detail"
                fi
            fi
        else
            # Positive / Problem test logic
            echo -e "${CYAN}Processing: $rel_path${NC}"
            
            run_with_timeout "$compile_timeout" "$TEJXC_BIN" "${COMPILER_ARGS[@]}" "$file" > "$compile_out" 2>&1
            local compile_exit=$?
            
            if [ $compile_exit -eq 0 ]; then
                if [ -f "$binary" ]; then
                    echo -e "  Running $filename..."
                    local timeout=$(runtime_timeout_for "$rel_path")
                    run_with_timeout "$timeout" "$binary" > "$out_file" 2>&1
                    local run_exit=$?
                    cat "$out_file"
                    
                    if [ $run_exit -eq 129 ] || [ $run_exit -eq 143 ] || [ $run_exit -eq 137 ]; then
                        echo -e "  ${RED}❌ RUNTIME TIMEOUT (exceeded ${timeout}s)${NC}"
                        test_passed=false
                        err_reason="Runtime timeout (exceeded ${timeout}s)"
                        echo "Timed out after ${timeout}s" > "$err_detail"
                    elif output_has_assertion_failure "$out_file"; then
                        if [ $run_exit -ne 0 ]; then
                            echo -e "  ${RED}❌ ASSERTION FAILURE${NC} (exit: $run_exit)"
                            err_reason="Assertion failed (exit: $run_exit)"
                        else
                            echo -e "  ${RED}❌ ASSERTION FAILURE${NC}"
                            err_reason="Assertion failed"
                        fi
                        test_passed=false
                        # Extract assertion failure details:
                        grep -E '❌ (FAIL|ASSERT FAILED)|AssertionFailed:|Expected:|Actual:' "$out_file" > "$err_detail"
                        [ ! -s "$err_detail" ] && tail -n 10 "$out_file" > "$err_detail"
                    elif [ $run_exit -eq 0 ]; then
                        echo -e "  ${GREEN}✅ PASS${NC}"
                        test_passed=true
                    else
                        echo -e "  ${RED}❌ RUNTIME ERROR${NC} (exit: $run_exit)"
                        test_passed=false
                        err_reason="Runtime error (exit: $run_exit)"
                        tail -n 15 "$out_file" > "$err_detail"
                    fi
                else
                    echo -e "  ${GREEN}✅ PASS${NC} (compiled + linked)"
                    test_passed=true
                fi
            else
                if [ $compile_exit -eq 129 ] || [ $compile_exit -eq 143 ] || [ $compile_exit -eq 137 ]; then
                    echo -e "  ${RED}❌ COMPILE TIMEOUT (exceeded ${compile_timeout}s)${NC}"
                    test_passed=false
                    err_reason="Compile timeout (exceeded ${compile_timeout}s)"
                    echo "Compilation timed out after ${compile_timeout}s" > "$err_detail"
                else
                    echo -e "  ${RED}❌ COMPILE ERROR${NC}"
                    cat "$compile_out"
                    test_passed=false
                    err_reason="Compilation failed"
                    cat "$compile_out" > "$err_detail"
                fi
            fi
        fi

        # Persist test outcome
        if [ "$test_passed" = true ]; then
            touch "$RESULTS_DIR/${id}.pass"
            rm -f "$err_detail"
        else
            touch "$RESULTS_DIR/${id}.fail"
            echo "$rel_path ($err_reason)" > "$RESULTS_DIR/${id}.err"
        fi
    ) > "$test_log" 2>&1

    # Cleanup test binaries and intermediate files
    rm -f "$binary" "$ll_file" "${file%.*}.o" "${file%.*}.s" "$compile_out" "$out_file"

    # Output log atomically so test outputs do not interleave
    cat "$test_log"
    rm -f "$test_log"
}

# --- Argument Parsing ---
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --positive) RUN_POSITIVE=true ;;
        --negative) RUN_NEGATIVE=true ;;
        --problems) RUN_PROBLEMS=true ;;
        --filter) shift; FILTER="$1" ;;
        --all) RUN_POSITIVE=true; RUN_NEGATIVE=true; RUN_PROBLEMS=true ;;
        -j|--jobs) shift; MAX_JOBS="$1" ;;
        -j*) MAX_JOBS="${1#-j}" ;;
        --jobs=*) MAX_JOBS="${1#*=}" ;;
        -s|--serial) MAX_JOBS=1 ;;
        --installed|--use-installed|-i)
            COMPILER_MODE="installed"
            DO_BUILD=false
            ;;
        --compiler)
            shift
            COMPILER_MODE="custom"
            TEJXC_BIN="$1"
            DO_BUILD=false
            ;;
        --compiler=*)
            COMPILER_MODE="custom"
            TEJXC_BIN="${1#*=}"
            DO_BUILD=false
            ;;
        --build)
            COMPILER_MODE="build"
            DO_BUILD=true
            ;;
        --no-build|--skip-build)
            DO_BUILD=false
            ;;
        --stdlib-path)
            shift
            CUSTOM_STDLIB="$1"
            ;;
        --runtime-path)
            shift
            CUSTOM_RUNTIME="$1"
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS] [TEST_PATHS...]"
            echo ""
            echo "Options:"
            echo "  --positive             Run positive tests"
            echo "  --negative             Run negative tests"
            echo "  --problems             Run problem tests"
            echo "  --all                  Run all tests (positive, negative, problems)"
            echo "  --filter <regex>       Filter test files by regex pattern"
            echo "  -j, --jobs <N>         Number of parallel test jobs (default: auto)"
            echo "  -s, --serial           Run tests serially (-j 1)"
            echo "  --build                Build compiler and runtime before testing (default)"
            echo "  --no-build             Skip building before testing"
            echo "  --installed, -i        Use installed tejxc compiler (~/.tejx/bin/tejxc or PATH)"
            echo "  --compiler <path>      Use custom tejxc binary path"
            echo "  --stdlib-path <path>   Override stdlib path"
            echo "  --runtime-path <path>  Override runtime library path"
            echo "  -h, --help             Show this help message"
            exit 0
            ;;
        *) SPECIFIC_PATHS+=("$1") ;;
    esac
    shift
done

if [ ${#SPECIFIC_PATHS[@]} -eq 0 ] && ! $RUN_POSITIVE && ! $RUN_NEGATIVE && ! $RUN_PROBLEMS; then
    RUN_POSITIVE=true
    RUN_NEGATIVE=true
    RUN_PROBLEMS=true
fi

# If TEJXC environment variable was provided and no explicit mode flag was given
if [ -n "$TEJXC_BIN" ] && [ "$COMPILER_MODE" = "build" ]; then
    COMPILER_MODE="custom"
    DO_BUILD=false
fi

# --- Compiler & Dependency Resolution ---
if [ "$COMPILER_MODE" = "installed" ]; then
    if command -v tejxc >/dev/null 2>&1; then
        TEJXC_BIN="$(command -v tejxc)"
    elif [ -x "$HOME/.tejx/bin/tejxc" ]; then
        TEJXC_BIN="$HOME/.tejx/bin/tejxc"
    elif [ -x "/usr/local/bin/tejxc" ]; then
        TEJXC_BIN="/usr/local/bin/tejxc"
    else
        echo -e "${RED}Error: Installed tejxc not found in PATH or \$HOME/.tejx/bin/tejxc${NC}" >&2
        exit 1
    fi
elif [ "$COMPILER_MODE" = "custom" ]; then
    if command -v "$TEJXC_BIN" >/dev/null 2>&1; then
        TEJXC_BIN="$(command -v "$TEJXC_BIN")"
    elif [ ! -x "$TEJXC_BIN" ]; then
        echo -e "${RED}Error: Specified compiler '$TEJXC_BIN' not found or not executable${NC}" >&2
        exit 1
    fi
else
    # Default: local build file
    TEJXC_BIN="$SCRIPT_DIR/target/release/tejxc"
fi

# Run build if requested (default is true)
if $DO_BUILD; then
    (cd "$SCRIPT_DIR" && ./build.sh) || exit 1
elif [ ! -x "$TEJXC_BIN" ]; then
    echo -e "${RED}Error: Compiler binary not found at $TEJXC_BIN${NC}" >&2
    echo -e "${YELLOW}Tip: Run without --no-build to compile it, or use --installed to test the installed compiler.${NC}" >&2
    exit 1
fi

# Resolve stdlib and runtime paths
if [ -n "$CUSTOM_STDLIB" ]; then
    STDLIB_PATH="$CUSTOM_STDLIB"
elif [ -d "$SCRIPT_DIR/src/library" ]; then
    STDLIB_PATH="$SCRIPT_DIR/src/library"
elif [ -d "$(dirname "$TEJXC_BIN")/../lib" ]; then
    STDLIB_PATH="$(dirname "$TEJXC_BIN")/../lib"
elif [ -d "$HOME/.tejx/lib" ]; then
    STDLIB_PATH="$HOME/.tejx/lib"
else
    STDLIB_PATH=""
fi

if [ -n "$CUSTOM_RUNTIME" ]; then
    RUNTIME_PATH="$CUSTOM_RUNTIME"
elif [ -f "$SCRIPT_DIR/target/release/tejx_rt.a" ]; then
    RUNTIME_PATH="$SCRIPT_DIR/target/release/tejx_rt.a"
elif [ -f "$SCRIPT_DIR/target/release/libtejx_rt.a" ]; then
    RUNTIME_PATH="$SCRIPT_DIR/target/release/libtejx_rt.a"
elif [ -f "$SCRIPT_DIR/target/debug/tejx_rt.a" ]; then
    RUNTIME_PATH="$SCRIPT_DIR/target/debug/tejx_rt.a"
elif [ -f "$(dirname "$TEJXC_BIN")/../runtime/tejx_rt.a" ]; then
    RUNTIME_PATH="$(dirname "$TEJXC_BIN")/../runtime/tejx_rt.a"
elif [ -f "$(dirname "$TEJXC_BIN")/runtime/tejx_rt.a" ]; then
    RUNTIME_PATH="$(dirname "$TEJXC_BIN")/runtime/tejx_rt.a"
elif [ -f "$HOME/.tejx/runtime/tejx_rt.a" ]; then
    RUNTIME_PATH="$HOME/.tejx/runtime/tejx_rt.a"
else
    RUNTIME_PATH=""
fi

COMPILER_ARGS=()
[ -n "$STDLIB_PATH" ] && [ -d "$STDLIB_PATH" ] && COMPILER_ARGS+=(--stdlib-path "$STDLIB_PATH")
[ -n "$RUNTIME_PATH" ] && [ -f "$RUNTIME_PATH" ] && COMPILER_ARGS+=(--runtime-path "$RUNTIME_PATH")

mkdir -p "$BUILD_DIR"
RESULTS_DIR=$(mktemp -d "$BUILD_DIR/results_XXXXXX" 2>/dev/null || mktemp -d)

# High-performance parallel semaphore via FIFO (event-driven, zero polling delay)
FIFO="$RESULTS_DIR/job_fifo"
mkfifo "$FIFO"
exec 3<>"$FIFO"
rm -f "$FIFO"

for ((i = 0; i < MAX_JOBS; i++)); do
    echo >&3
done

cleanup() {
    trap - INT TERM EXIT
    kill $(jobs -p) 2>/dev/null
    exec 3>&- 2>/dev/null
    find "$BUILD_DIR" -type f \( -name "*.ll" -o -name "*.tmp" -o -name "*.s.tmp" -o -name "*.o.tmp" -o -name "*.o" -o -name "*.s" \) -delete 2>/dev/null
    find "$SCRIPT_DIR/tests" -type f \( -name "*.ll" -o -name "*.tmp" -o -name "*.s.tmp" -o -name "*.o.tmp" -o -name "*.o" -o -name "*.s" \) -delete 2>/dev/null
    rm -rf "$RESULTS_DIR" 2>/dev/null
    exit 1
}
trap cleanup INT TERM

run_parallel_test() {
    local file=$1
    local type=$2
    local id=$3

    read -u 3
    (
        run_test_file "$file" "$type" "$id"
        echo >&3
    ) &
}

TEST_ID=0

if [ ${#SPECIFIC_PATHS[@]} -gt 0 ]; then
    print_header "Selective Tests"
    for path in "${SPECIFIC_PATHS[@]}"; do
        if [ -d "$path" ]; then
            while read -r f; do
                [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
                type="positive"
                [[ "$f" == *"negative"* ]] && type="negative"
                [[ "$f" == *"problems"* ]] && type="problem"
                run_parallel_test "$f" "$type" "$TEST_ID"
                TEST_ID=$((TEST_ID + 1))
            done < <(find "$path" -name "*.tx" | sort)
        elif [ -f "$path" ]; then
            [[ -n "$FILTER" && ! "$path" =~ $FILTER ]] && continue
            type="positive"
            [[ "$path" == *"negative"* ]] && type="negative"
            [[ "$path" == *"problems"* ]] && type="problem"
            run_parallel_test "$path" "$type" "$TEST_ID"
            TEST_ID=$((TEST_ID + 1))
        else
            echo -e "${RED}Error: Path not found: $path${NC}"
        fi
    done
else
    if $RUN_POSITIVE; then
        print_header "Positive Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_parallel_test "$f" "positive" "$TEST_ID"
            TEST_ID=$((TEST_ID + 1))
        done < <(find "$SCRIPT_DIR/tests/positive" -name "*.tx" -not -path "*/modules/*" | sort)
    fi

    if $RUN_NEGATIVE; then
        print_header "Negative Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_parallel_test "$f" "negative" "$TEST_ID"
            TEST_ID=$((TEST_ID + 1))
        done < <(find "$SCRIPT_DIR/tests/negative" -name "*.tx" | sort)
    fi

    if $RUN_PROBLEMS; then
        print_header "Problem Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_parallel_test "$f" "problem" "$TEST_ID"
            TEST_ID=$((TEST_ID + 1))
        done < <(find "$SCRIPT_DIR/tests/problems" -name "*.tx" | sort)
    fi
fi

# Wait for all workers to finish
wait
exec 3>&- 2>/dev/null

# --- Summary ---
PASSED=$(ls -1 "$RESULTS_DIR"/*.pass 2>/dev/null | wc -l | tr -d ' ')
FAILED=$(ls -1 "$RESULTS_DIR"/*.fail 2>/dev/null | wc -l | tr -d ' ')

echo ""
echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   Final Results Summary${NC}"
echo -e "${CYAN}============================================${NC}"
echo -e "  Passed: ${GREEN}$PASSED${NC}"
if [ "$FAILED" -gt 0 ]; then
    echo -e "  Failed: ${RED}$FAILED${NC}"
    echo -e "\n${RED}============================================${NC}"
    echo -e "${RED}   Failures Detail (${FAILED} failed):${NC}"
    echo -e "${RED}============================================${NC}"
    for err_f in "$RESULTS_DIR"/*.err; do
        if [ -f "$err_f" ]; then
            err_summary=$(cat "$err_f")
            base_id=$(basename "${err_f%.err}")
            detail_f="$RESULTS_DIR/${base_id}.detail"
            echo -e "${RED}❌ $err_summary${NC}"
            if [ -f "$detail_f" ] && [ -s "$detail_f" ]; then
                echo -e "   ${YELLOW}Error details:${NC}"
                sed 's/^/     /' "$detail_f"
            fi
            echo ""
        fi
    done
else
    echo -e "  Failed: ${GREEN}0${NC}"
fi

rm -rf "$RESULTS_DIR"
find "$BUILD_DIR" -type f \( -name "*.ll" -o -name "*.tmp" -o -name "*.s.tmp" -o -name "*.o.tmp" -o -name "*.o" -o -name "*.s" \) -delete 2>/dev/null
find "$SCRIPT_DIR/tests" -type f \( -name "*.ll" -o -name "*.tmp" -o -name "*.s.tmp" -o -name "*.o.tmp" -o -name "*.o" -o -name "*.s" \) -delete 2>/dev/null

[ "$FAILED" -eq 0 ] && exit 0 || exit 1
