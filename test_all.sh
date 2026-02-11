#!/bin/bash

# ============================================
# TejX Rust Compiler - Test All Script
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$SCRIPT_DIR/tests"
BUILD_DIR="$SCRIPT_DIR/build"
TEJXR_BIN="$SCRIPT_DIR/target/release/tejxr"

# Track results
PASSED=0
FAILED=0
ERRORS=()

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   TejX Rust Compiler - Test Suite${NC}"
echo -e "${CYAN}============================================${NC}"
echo ""

# 1. Build the compiler
echo -e "${YELLOW}>>> Building TejX Compiler...${NC}"
./build.sh

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Compiler Build Failed!${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Compiler Build Successful.${NC}"
echo ""

# 2. Create build directory
mkdir -p "$BUILD_DIR/tests"

# 3. Run all test files
echo -e "${YELLOW}>>> Running All Tests...${NC}"
echo "----------------------------------------"

for FILE in "$TESTS_DIR"/*.tx; do
    [ -f "$FILE" ] || continue
    
    FILENAME=$(basename "${FILE%.*}")
    echo -e "${CYAN}Processing: $FILENAME${NC}"
    
    # Run the Rust TejX compiler
    "$TEJXR_BIN" "$FILE" 2>&1
    COMPILE_EXIT=$?
    
    if [ $COMPILE_EXIT -eq 0 ]; then
        # Check if binary was created
        BINARY="${FILE%.*}"
        
        if [ -f "$BINARY" ]; then
            # Move binary to build folder
            mv "$BINARY" "$BUILD_DIR/tests/$FILENAME"
            
            # Run the binary
            echo -e "  Running $FILENAME..."
            OUTPUT=$("$BUILD_DIR/tests/$FILENAME" 2>&1)
            RUN_EXIT=$?
            
            if [ $RUN_EXIT -eq 0 ]; then
                echo -e "  ${GREEN}✅ PASS${NC} (exit: $RUN_EXIT)"
                if [ -n "$OUTPUT" ]; then
                    echo "  Output: $OUTPUT"
                fi
                PASSED=$((PASSED + 1))
            else
                echo -e "  ${RED}❌ RUNTIME ERROR${NC} (exit: $RUN_EXIT)"
                if [ -n "$OUTPUT" ]; then
                    echo "  Output: $OUTPUT"
                fi
                FAILED=$((FAILED + 1))
                ERRORS+=("$FILENAME (runtime error, exit: $RUN_EXIT)")
            fi
        else
            # Check if .ll file exists (compilation succeeded but linking failed)
            LL_FILE="${FILE%.*}.ll"
            if [ -f "$LL_FILE" ]; then
                echo -e "  ${YELLOW}⚠️  LINK ERROR${NC} (LLVM IR generated, clang linking failed)"
                mv "$LL_FILE" "$BUILD_DIR/tests/${FILENAME}.ll"
                FAILED=$((FAILED + 1))
                ERRORS+=("$FILENAME (linking failed)")
            else
                echo -e "  ${GREEN}✅ PASS${NC} (compiled + linked)"
                PASSED=$((PASSED + 1))
            fi
        fi
    else
        # Check if .ll was left behind
        LL_FILE="${FILE%.*}.ll"
        if [ -f "$LL_FILE" ]; then
            mv "$LL_FILE" "$BUILD_DIR/tests/${FILENAME}.ll"
            echo -e "  ${YELLOW}⚠️  LINK ERROR${NC} (LLVM IR saved to build/tests/${FILENAME}.ll)"
        else
            echo -e "  ${RED}❌ COMPILE ERROR${NC}"
        fi
        FAILED=$((FAILED + 1))
        ERRORS+=("$FILENAME (compilation failed)")
    fi
    
    # Clean up any leftover .ll files
    [ -f "${FILE%.*}.ll" ] && rm "${FILE%.*}.ll"
    
    echo "----------------------------------------"
done

# 4. Summary
TOTAL=$((PASSED + FAILED))
echo ""
echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   Test Results Summary${NC}"
echo -e "${CYAN}============================================${NC}"
echo -e "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASSED${NC}"
echo -e "  ${RED}Failed: $FAILED${NC}"

if [ ${#ERRORS[@]} -gt 0 ]; then
    echo ""
    echo -e "${RED}Failed tests:${NC}"
    for err in "${ERRORS[@]}"; do
        echo -e "  - $err"
    done
fi

echo ""
if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}>>> All Tests Passed! <<<${NC}"
else
    echo -e "${YELLOW}>>> $FAILED test(s) failed <<<${NC}"
fi
