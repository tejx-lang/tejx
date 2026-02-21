#!/bin/bash

# ============================================
# TejX Unified Test Runner
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEJXC_BIN="$SCRIPT_DIR/target/release/tejxc"
BUILD_DIR="$SCRIPT_DIR/build/tests"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Results
PASSED=0
FAILED=0
ERRORS=()

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
    (sleep "$timeout"; kill -SIGHUP "$child_pid" 2>/dev/null) >/dev/null 2>&1 &
    local watcher_pid=$!
    wait "$child_pid" 2>/dev/null
    local exit_code=$?
    kill "$watcher_pid" 2>/dev/null
    return "$exit_code"
}

print_header() {
    echo -e "${CYAN}============================================${NC}"
    echo -e "${CYAN}   TejX Test Runner: $1${NC}"
    echo -e "${CYAN}============================================${NC}"
}

run_test_file() {
    local file=$1
    local type=$2 # positive, negative, problem
    local rel_path="${file#$SCRIPT_DIR/}"
    local filename=$(basename "${file%.*}")
    
    echo "----------------------------------------"
    if [ "$type" == "negative" ]; then
        echo -e "${YELLOW}Testing: $rel_path${NC}"
        
        local out_file=$(mktemp)
        local binary="${file%.*}"
        local ll_file="${file%.*}.ll"
        rm -f "$binary" "$ll_file"

        # Read expected failure type and description
        local expected_type="COMPILE_ERROR"
        grep -q "EXPECTED: RUNTIME_ERROR" "$file" && expected_type="RUNTIME_ERROR"
        local description=$(grep "Description:" "$file" | cut -d ':' -f 2- | xargs)
        
        echo -e "  Description: $description"
        echo -e "  Expected:    ${CYAN}$expected_type${NC}"
        
        run_with_timeout 5 "$TEJXC_BIN" "$file" > "$out_file" 2>&1
        local compile_exit=$?
        
        local actual=""
        if [ $compile_exit -eq 129 ] || [ $compile_exit -eq 143 ]; then
            actual="COMPILE_TIMEOUT (HANG)"
        elif [ $compile_exit -ne 0 ]; then
            actual="COMPILE_ERROR"
        else
            actual="COMPILE_SUCCESS"
        fi

        if [ "$expected_type" == "COMPILE_ERROR" ]; then
            if [ "$actual" == "COMPILE_ERROR" ] || [ "$actual" == "COMPILE_TIMEOUT (HANG)" ]; then
                echo -e "  Actual:      ${GREEN}$actual${NC}"
                echo -e "  Error Log:   $(grep -v "Terminated: 15" "$out_file" | grep -v "sleep" | head -n 2 | tr '\n' ' ')"
                echo -e "  Result:      ${GREEN}✅ PASS${NC}"
                PASSED=$((PASSED + 1))
            else
                echo -e "  Actual:      ${RED}$actual${NC}"
                echo -e "  Result:      ${RED}❌ FAIL${NC}"
                FAILED=$((FAILED + 1))
                ERRORS+=("$rel_path (Expected COMPILE_ERROR but got $actual)")
            fi
        else # EXPECTED: RUNTIME_ERROR
            if [ "$actual" == "COMPILE_SUCCESS" ]; then
                if [ -f "$binary" ]; then
                    run_with_timeout 2 "$binary" > "$out_file" 2>&1
                    local run_exit=$?
                    
                    local actual_runtime=""
                    if [ $run_exit -eq 129 ] || [ $run_exit -eq 143 ]; then
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
                        PASSED=$((PASSED + 1))
                    else
                        echo -e "  Result:      ${RED}❌ FAIL${NC}"
                        FAILED=$((FAILED + 1))
                        ERRORS+=("$rel_path (Expected RUNTIME_ERROR but got $actual_runtime)")
                    fi
                else
                    echo -e "  Actual:      ${RED}NO_BINARY_GENERATED${NC}"
                    echo -e "  Result:      ${RED}❌ FAIL${NC}"
                    FAILED=$((FAILED + 1))
                    ERRORS+=("$rel_path (No binary generated)")
                fi
            else
                echo -e "  Actual:      ${RED}$actual${NC} (Caught too early?)"
                echo -e "  Result:      ${RED}❌ FAIL${NC}"
                FAILED=$((FAILED + 1))
                ERRORS+=("$rel_path (Expected RUNTIME_ERROR but failed to compile)")
            fi
        fi
        rm -f "$out_file" "$binary" "$ll_file"
    else
        # Positive / Problem test logic
        echo -e "${CYAN}Processing: $rel_path${NC}"
        
        local out_file=$(mktemp)
        local binary="${file%.*}"
        local ll_file="${file%.*}.ll"
        rm -f "$binary" "$ll_file"

        "$TEJXC_BIN" "$file" 2>&1
        local compile_exit=$?
        
        if [ $compile_exit -eq 0 ]; then
            if [ -f "$binary" ]; then
                echo -e "  Running $filename..."
                "$binary" 2>&1 | tee "$out_file"
                local run_exit=${PIPESTATUS[0]}
                
                if [ $run_exit -eq 0 ]; then
                    echo -e "  ${GREEN}✅ PASS${NC}"
                    PASSED=$((PASSED + 1))
                else
                    echo -e "  ${RED}❌ RUNTIME ERROR${NC} (exit: $run_exit)"
                    FAILED=$((FAILED + 1))
                    ERRORS+=("$rel_path (Runtime error)")
                fi
            else
                echo -e "  ${GREEN}✅ PASS${NC} (compiled + linked)"
                PASSED=$((PASSED + 1))
            fi
        else
            echo -e "  ${RED}❌ COMPILE ERROR${NC}"
            FAILED=$((FAILED + 1))
            ERRORS+=("$rel_path (Compilation failed)")
        fi
        rm -f "$out_file" "$binary" "$ll_file"
    fi
}

# --- Argument Parsing ---
if [ "$#" -eq 0 ]; then
    RUN_POSITIVE=true
    RUN_NEGATIVE=true
    RUN_PROBLEMS=true
fi

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --positive) RUN_POSITIVE=true ;;
        --negative) RUN_NEGATIVE=true ;;
        --problems) RUN_PROBLEMS=true ;;
        --filter) shift; FILTER="$1" ;;
        --all) RUN_POSITIVE=true; RUN_NEGATIVE=true; RUN_PROBLEMS=true ;;
        *) SPECIFIC_PATHS+=("$1") ;;
    esac
    shift
done

# --- Execution ---
[ ! -f "$TEJXC_BIN" ] && ./build.sh

if [ ${#SPECIFIC_PATHS[@]} -gt 0 ]; then
    print_header "Selective Tests"
    for path in "${SPECIFIC_PATHS[@]}"; do
        if [ -d "$path" ]; then
            while read -r f; do
                [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
                type="positive"
                [[ "$f" == *"negative"* ]] && type="negative"
                [[ "$f" == *"problems"* ]] && type="problem"
                run_test_file "$f" "$type"
            done < <(find "$path" -name "*.tx" | sort)
        elif [ -f "$path" ]; then
            [[ -n "$FILTER" && ! "$path" =~ $FILTER ]] && continue
            type="positive"
            [[ "$path" == *"negative"* ]] && type="negative"
            [[ "$path" == *"problems"* ]] && type="problem"
            run_test_file "$path" "$type"
        else
            echo -e "${RED}Error: Path not found: $path${NC}"
        fi
    done
else
    if $RUN_POSITIVE; then
        print_header "Positive Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_test_file "$f" "positive"
        done < <(find "$SCRIPT_DIR/tests/positive" -name "*.tx" -not -path "*/modules/*" | sort)
    fi

    if $RUN_NEGATIVE; then
        print_header "Negative Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_test_file "$f" "negative"
        done < <(find "$SCRIPT_DIR/tests/negative" -name "*.tx" | sort)
    fi

    if $RUN_PROBLEMS; then
        print_header "Problem Tests"
        while read -r f; do 
            [[ -n "$FILTER" && ! "$f" =~ $FILTER ]] && continue
            run_test_file "$f" "problem"
        done < <(find "$SCRIPT_DIR/tests/problems" -name "*.tx" | sort)
    fi
fi

# --- Summary ---
echo ""
echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   Final Results Summary${NC}"
echo -e "${CYAN}============================================${NC}"
echo -e "  Passed: ${GREEN}$PASSED${NC}"
echo -e "  Failed: ${RED}$FAILED${NC}"

if [ ${#ERRORS[@]} -gt 0 ]; then
    echo -e "\n${RED}Failures:${NC}"
    for err in "${ERRORS[@]}"; do echo -e "  - $err"; done
fi

[ $FAILED -eq 0 ] && exit 0 || exit 1
