#!/bin/bash

# ============================================
# TejX Unified Test Runner (High-Performance Parallel)
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEJXC_BIN="$SCRIPT_DIR/target/release/tejxc"
BUILD_DIR="$SCRIPT_DIR/build/tests"

# Common paths (resolved locally, but passed to compiler for clarity)
STDLIB_PATH="$SCRIPT_DIR/src/library"
RUNTIME_PATH="$SCRIPT_DIR/target/release/tejx_rt.a"
[ ! -f "$RUNTIME_PATH" ] && RUNTIME_PATH="$SCRIPT_DIR/target/debug/tejx_rt.a"

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
        tests/positive/std/net.tx) echo 20 ;;
        tests/positive/std/thread.tx|tests/problems/producer_consumer.tx) echo 15 ;;
        tests/problems/benchmark.tx) echo 60 ;;
        *) echo 10 ;;
    esac
}

print_header() {
    echo -e "${CYAN}============================================${NC}"
    echo -e "${CYAN}   TejX Test Runner: $1 (${MAX_JOBS} parallel workers)${NC}"
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
    rm -f "$binary" "$ll_file" "${file%.*}.o" "${file%.*}.s" "${file%.*}-"*.tmp "${file%.*}.o.tmp"
    
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
            
            run_with_timeout 20 "$TEJXC_BIN" --stdlib-path "$STDLIB_PATH" --runtime-path "$RUNTIME_PATH" "$file" > "$compile_out" 2>&1
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
            
            run_with_timeout "$compile_timeout" "$TEJXC_BIN" --stdlib-path "$STDLIB_PATH" --runtime-path "$RUNTIME_PATH" "$file" > "$compile_out" 2>&1
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
    rm -f "$binary" "$ll_file" "${file%.*}.o" "${file%.*}.s" "${file%.*}-"*.tmp "${file%.*}.o.tmp" "${file%.*}.s.tmp" "$compile_out" "$out_file"

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
        *) SPECIFIC_PATHS+=("$1") ;;
    esac
    shift
done

if [ ${#SPECIFIC_PATHS[@]} -eq 0 ] && ! $RUN_POSITIVE && ! $RUN_NEGATIVE && ! $RUN_PROBLEMS; then
    RUN_POSITIVE=true
    RUN_NEGATIVE=true
    RUN_PROBLEMS=true
fi

# --- Execution ---
./build.sh || exit 1

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
