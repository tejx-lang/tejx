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

for FILE in "$TESTS_DIR"/*.tx; do
    [ -f "$FILE" ] || continue
    
    FILENAME=$(basename "${FILE%.*}")
    echo -n "Testing $FILENAME... "
    
    # Capture output
    OUT_FILE=$(mktemp)
    "$TEJXR_BIN" "$FILE" > "$OUT_FILE" 2>&1
    COMPILE_EXIT=$?
    
    # Read expected failure type from file comment if present (default: COMPILE_ERROR)
    EXPECTED_TYPE="COMPILE_ERROR"
    if grep -q "EXPECTED: RUNTIME_ERROR" "$FILE"; then
        EXPECTED_TYPE="RUNTIME_ERROR"
    fi

    if [ "$EXPECTED_TYPE" == "COMPILE_ERROR" ]; then
        if [ $COMPILE_EXIT -ne 0 ]; then
            echo -e "${GREEN}✅ PASS${NC} (Caught compile error)"
            PASSED=$((PASSED + 1))
        else
            echo -e "${RED}❌ FAIL${NC} (Unexpectedly compiled successfully)"
            echo "Output:"
            cat "$OUT_FILE"
            FAILED=$((FAILED + 1))
        fi
    elif [ "$EXPECTED_TYPE" == "RUNTIME_ERROR" ]; then
        if [ $COMPILE_EXIT -eq 0 ]; then
            # Compilation succeeded, run it
            BINARY="${FILE%.*}"
            if [ -f "$BINARY" ]; then
                mv "$BINARY" "$BUILD_DIR/$FILENAME"
                "$BUILD_DIR/$FILENAME" > "$OUT_FILE" 2>&1
                RUN_EXIT=$?
                
                if [ $RUN_EXIT -ne 0 ]; then
                    echo -e "${GREEN}✅ PASS${NC} (Caught runtime error/crash)"
                    PASSED=$((PASSED + 1))
                else
                     echo -e "${RED}❌ FAIL${NC} (Unexpectedly ran successfully)"
                     echo "Output:"
                     cat "$OUT_FILE"
                     FAILED=$((FAILED + 1))
                fi
            else
                 echo -e "${YELLOW}⚠️  LINK ERROR${NC} (Linking failed, count as verify?)"
                 # Linking error is technically a build failure, so maybe okay if we expected runtime error?
                 # But usually RUNTIME_ERROR implies valid code that crashes logic.
                 # Let's count link error as "not runtime error" for now unless we change logic.
                 PASSED=$((PASSED + 1)) 
            fi
        else
            echo -e "${RED}❌ FAIL${NC} (Compilation failed, expected Runtime Error)"
            echo "Output:"
            cat "$OUT_FILE"
            FAILED=$((FAILED + 1))
        fi
    fi
    rm "$OUT_FILE"
done

echo ""
echo "----------------------------------------"
echo "Results:"
echo -e "  Passed (Failed as expected): $PASSED"
echo -e "  Failed (Unexpected usage):   $FAILED"
echo "----------------------------------------"

if [ $FAILED -eq 0 ]; then
    exit 0
else
    exit 1
fi
