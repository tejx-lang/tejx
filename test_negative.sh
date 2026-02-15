#!/bin/bash

# ============================================
# TejX Rust Compiler - Negative Test Suite
# ============================================
# Runs tests that are EXPECTED to fail compilation or runtime.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$SCRIPT_DIR/tests/negative"
BUILD_DIR="$SCRIPT_DIR/build/tests/negative"
TEJXR_BIN="$SCRIPT_DIR/target/release/tejxr"

# Track results
PASSED=0
FAILED=0

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   TejX Negative Test Suite${NC}"
echo -e "${CYAN}   (Tests SHOULD fail to pass)${NC}"
echo -e "${CYAN}============================================${NC}"
echo ""

# Ensure compiler is built (fast check)
if [ ! -f "$TEJXR_BIN" ]; then
    echo "Compiler binary not found. Building..."
    ./build.sh
fi

mkdir -p "$BUILD_DIR"

echo -e "${YELLOW}>>> Running Negative Tests...${NC}"
echo "----------------------------------------"

# Helper function for timeout (macOS compatibility)
run_with_timeout() {
    local timeout=$1
    shift
    "$@" &
    local child_pid=$!
    (sleep "$timeout"; kill -SIGHUP "$child_pid" 2>/dev/null) &
    local watcher_pid=$!
    wait "$child_pid" 2>/dev/null
    local exit_code=$?
    kill "$watcher_pid" 2>/dev/null
    return "$exit_code"
}

for FILE in "$TESTS_DIR"/*.tx; do
    [ -f "$FILE" ] || continue
    
    FILENAME=$(basename "${FILE%.*}")
    echo -e "${CYAN}----------------------------------------${NC}"
    echo -e "${YELLOW}Testing: $FILENAME${NC}"
    
    # Capture output
    OUT_FILE=$(mktemp)
    
    # Read expected failure type and description
    EXPECTED_TYPE="COMPILE_ERROR"
    if grep -q "EXPECTED: RUNTIME_ERROR" "$FILE"; then
        EXPECTED_TYPE="RUNTIME_ERROR"
    fi
    DESCRIPTION=$(grep "Description:" "$FILE" | cut -d ':' -f 2- | xargs)
    
    echo -e "  Description: $DESCRIPTION"
    echo -e "  Expected:    ${CYAN}$EXPECTED_TYPE${NC}"
    
    # Pre-test cleanup
    BINARY="${FILE%.*}"
    LL_FILE="${FILE%.*}.ll"
    rm -f "$BINARY" "$LL_FILE"
    
    # Attempt compilation with a timeout
    run_with_timeout 5 "$TEJXR_BIN" "$FILE" > "$OUT_FILE" 2>&1
    COMPILE_EXIT=$?
    
    if [ $COMPILE_EXIT -eq 129 ] || [ $COMPILE_EXIT -eq 143 ]; then
        ACTUAL="COMPILE_TIMEOUT (HANG)"
    elif [ $COMPILE_EXIT -ne 0 ]; then
        ACTUAL="COMPILE_ERROR"
    else
        ACTUAL="COMPILE_SUCCESS"
    fi

    if [ "$EXPECTED_TYPE" == "COMPILE_ERROR" ]; then
        if [ "$ACTUAL" == "COMPILE_ERROR" ] || [ "$ACTUAL" == "COMPILE_TIMEOUT (HANG)" ]; then
            echo -e "  Actual:      ${GREEN}$ACTUAL${NC}"
            echo -e "  Error Log:   $(grep -v "Terminated: 15" "$OUT_FILE" | grep -v "sleep" | head -n 2 | tr '\n' ' ')"
            echo -e "  Result:      ${GREEN}✅ PASS${NC}"
            PASSED=$((PASSED + 1))
        else
            echo -e "  Actual:      ${RED}$ACTUAL${NC}"
            echo -e "  Result:      ${RED}❌ FAIL${NC}"
            if [ "$ACTUAL" == "COMPILE_SUCCESS" ]; then
                echo -e "  Output Snippet:"
                cat "$OUT_FILE" | head -n 3 | sed 's/^/    /'
            fi
            FAILED=$((FAILED + 1))
        fi
    elif [ "$EXPECTED_TYPE" == "RUNTIME_ERROR" ]; then
        if [ "$ACTUAL" == "COMPILE_SUCCESS" ]; then
            if [ -f "$BINARY" ]; then
                mv "$BINARY" "$BUILD_DIR/$FILENAME"
                run_with_timeout 2 "$BUILD_DIR/$FILENAME" > "$OUT_FILE" 2>&1
                RUN_EXIT=$?
                
                if [ $RUN_EXIT -eq 129 ] || [ $RUN_EXIT -eq 143 ]; then
                    ACTUAL_RUNTIME="RUNTIME_TIMEOUT (HANG)"
                elif [ $RUN_EXIT -ne 0 ]; then
                    ACTUAL_RUNTIME="RUNTIME_ERROR (CRASH)"
                    echo -e "  Runtime Log: $(head -n 2 "$OUT_FILE" | tr '\n' ' ')"
                else
                    ACTUAL_RUNTIME="RUNTIME_SUCCESS"
                fi
                
                echo -e "  Actual:      ${YELLOW}$ACTUAL_RUNTIME${NC}"
                if [[ "$ACTUAL_RUNTIME" == *"ERROR"* ]] || [ $RUN_EXIT -eq 139 ] || [ $RUN_EXIT -eq 134 ] || [ $RUN_EXIT -ne 0 ]; then
                    echo -e "  Result:      ${GREEN}✅ PASS${NC}"
                    PASSED=$((PASSED + 1))
                else
                    echo -e "  Result:      ${RED}❌ FAIL${NC}"
                    FAILED=$((FAILED + 1))
                fi
            else
                # Check for .ll file
                if [ -f "$LL_FILE" ]; then
                    echo -e "  Actual:      ${RED}LINKER_ERROR${NC}"
                    echo -e "  Result:      ${RED}❌ FAIL${NC}"
                else
                    echo -e "  Actual:      ${RED}NO_BINARY_GENERATED${NC}"
                    echo -e "  Result:      ${RED}❌ FAIL${NC}"
                fi
                FAILED=$((FAILED + 1))
            fi
        else
            echo -e "  Actual:      ${RED}$ACTUAL${NC} (Caught too early?)"
            echo -e "  Error Log:   $(grep -v "Terminated: 15" "$OUT_FILE" | grep -v "sleep" | head -n 2 | tr '\n' ' ')"
            echo -e "  Result:      ${RED}❌ FAIL${NC}"
            FAILED=$((FAILED + 1))
        fi
    fi
    
    # Cleanup artifacts
    rm -f "$OUT_FILE"
    rm -f "$BINARY"
    rm -f "$LL_FILE"
    rm -f "$BUILD_DIR/$FILENAME"
done

echo -e "${CYAN}----------------------------------------${NC}"
echo -e "${CYAN}Final Results Summary:${NC}"
echo -e "  ${GREEN}Passed (Failed as expected): $PASSED${NC}"
echo -e "  ${RED}Failed (Unexpected outcome): $FAILED${NC}"
echo -e "${CYAN}----------------------------------------${NC}"

if [ $FAILED -eq 0 ]; then
    exit 0
else
    exit 1
fi
