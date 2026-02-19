#!/bin/bash

# ============================================
# TejX Rust Compiler - Test Problems Script
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$SCRIPT_DIR/tests/problems"
BUILD_DIR="$SCRIPT_DIR/build/problems"
TEJXC_BIN="$SCRIPT_DIR/target/release/tejxc"

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
echo -e "${CYAN}   TejX Rust Compiler - Problems Suite${NC}"
echo -e "${CYAN}============================================${NC}"
echo ""

# 1. Build the compiler (ensure it is up to date)
# ./build.sh # Skip full build, assume it's built or use existing

if [ ! -f "$TEJXC_BIN" ]; then
    echo -e "${RED}❌ Compiler binary not found at $TEJXC_BIN${NC}"
    echo "Please run ./build.sh first."
    exit 1
fi

# 2. Create build directory
mkdir -p "$BUILD_DIR"

# 3. Run problem files
echo -e "${YELLOW}>>> Running Problem Tests...${NC}"
echo "----------------------------------------"

PROBLEMS_TO_RUN=()
if [ "$#" -gt 0 ]; then
    # Use provided arguments
    for arg in "$@"; do
        # Handle simple filenames vs paths
        if [[ "$arg" == *.tx ]]; then
            if [ -f "$TESTS_DIR/$arg" ]; then
                PROBLEMS_TO_RUN+=("$TESTS_DIR/$arg")
            elif [ -f "$arg" ]; then
                PROBLEMS_TO_RUN+=("$arg")
            else
                echo -e "${RED}File not found: $arg${NC}"
            fi
        else
            # Try appending extension
             if [ -f "$TESTS_DIR/$arg.tx" ]; then
                PROBLEMS_TO_RUN+=("$TESTS_DIR/$arg.tx")
            fi
        fi
    done
else
    # Default: Find all .tx files
    while read -r FILE; do
        PROBLEMS_TO_RUN+=("$FILE")
    done < <(find "$TESTS_DIR" -name "*.tx" | sort)
fi

for FILE in "${PROBLEMS_TO_RUN[@]}"; do
    [ -f "$FILE" ] || continue
    
    REL_PATH="${FILE#$TESTS_DIR/}"
    FILENAME=$(basename "${FILE%.*}")
    
    echo -e "${CYAN}Processing: $REL_PATH${NC}"
    
    # Run the Rust TejX compiler
    "$TEJXC_BIN" "$FILE" 2>&1
    COMPILE_EXIT=$?
    
    if [ $COMPILE_EXIT -eq 0 ]; then
        # Check if binary was created
        BINARY="${FILE%.*}"
        
        if [ -f "$BINARY" ]; then
            # Move binary to build folder
            mv "$BINARY" "$BUILD_DIR/$FILENAME"
            
            # Run the binary
            echo -e "  Running $FILENAME..."
            # Create a temporary file for output
            OUT_FILE=$(mktemp)
            "$BUILD_DIR/$FILENAME" 2>&1 | tee "$OUT_FILE"
            RUN_EXIT=${PIPESTATUS[0]}
            # OUTPUT=$(cat "$OUT_FILE") # Unused
            rm "$OUT_FILE"
            
            if [ $RUN_EXIT -eq 0 ]; then
                echo -e "  ${GREEN}✅ PASS${NC} (exit: $RUN_EXIT)"
                PASSED=$((PASSED + 1))
            else
                echo -e "  ${RED}❌ RUNTIME ERROR${NC} (exit: $RUN_EXIT)"
                FAILED=$((FAILED + 1))
                ERRORS+=("$REL_PATH (runtime error, exit: $RUN_EXIT)")
            fi
        else
             # Check if .ll file exists (compilation succeeded but linking failed)
            LL_FILE="${FILE%.*}.ll"
            if [ -f "$LL_FILE" ]; then
                echo -e "  ${YELLOW}⚠️  LINK ERROR${NC} (LLVM IR generated, clang linking failed)"
                mv "$LL_FILE" "$BUILD_DIR/${FILENAME}.ll"
                FAILED=$((FAILED + 1))
                ERRORS+=("$REL_PATH (linking failed)")
            else
                echo -e "  ${GREEN}✅ PASS${NC} (compiled + linked)"
                PASSED=$((PASSED + 1))
            fi
        fi
    else
        # Check if .ll was left behind
        LL_FILE="${FILE%.*}.ll"
        if [ -f "$LL_FILE" ]; then
            mv "$LL_FILE" "$BUILD_DIR/${FILENAME}.ll"
            echo -e "  ${YELLOW}⚠️  LINK ERROR${NC} (LLVM IR saved to build/problems/${FILENAME}.ll)"
        else
            echo -e "  ${RED}❌ COMPILE ERROR${NC}"
        fi
        FAILED=$((FAILED + 1))
        ERRORS+=("$REL_PATH (compilation failed)")
    fi
    
    # Clean up any leftover .ll files
    [ -f "${FILE%.*}.ll" ] && rm "${FILE%.*}.ll"
    
    echo "----------------------------------------"
done

# 4. Summary
TOTAL=$((PASSED + FAILED))
echo ""
echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}   Problem Test Results Summary${NC}"
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
    echo -e "${GREEN}>>> All Problem Tests Passed! <<<${NC}"
else
    echo -e "${YELLOW}>>> $FAILED test(s) failed <<<${NC}"
fi
