#!/bin/bash

# Create build directory if it doesn't exist
mkdir -p build/examples

# 1. Compile the Compiler
echo ">>> Compiling TejX Compiler (tejxc)..."
SOURCES="src/main.cpp src/lexer/Lexer.cpp src/parser/Parser.cpp src/codegen/CodeGen.cpp"
clang++ -std=c++17 -Iinclude $SOURCES -o build/tejxc

if [ $? -ne 0 ]; then
    echo "Compiler Compilation Failed!"
    exit 1
fi
echo "Compiler Build Successful."

# 2. Build and Run All Examples
echo ""
echo ">>> Running All Examples..."
echo "----------------------------------------"

for FILE in tests/*.tx; do
    echo "Processing: $FILE"
    
    # Run TejX Compiler
    ./build/tejxc "$FILE"
    
    if [ $? -eq 0 ]; then
        # Determine output binary name
        FILENAME=$(basename "${FILE%.*}")
        OLD_BINARY="${FILE%.*}"
        NEW_BINARY="build/tests/$FILENAME"
        
        # Move binary to build folder
        if [ -f "$OLD_BINARY" ]; then
            mkdir -p build/tests
            mv "$OLD_BINARY" "$NEW_BINARY"
        fi
        
        # Move .cpp file to build folder
        if [ -f "${OLD_BINARY}.cpp" ]; then
            mkdir -p build/tests
            mv "${OLD_BINARY}.cpp" "build/tests/${FILENAME}.cpp"
        fi
        
        if [ -f "$NEW_BINARY" ]; then
            echo "Running $NEW_BINARY..."
            "$NEW_BINARY"
        else
            echo "Error: Binary not found for $FILE"
        fi
    else
        echo "Error: TejX Compilation failed for $FILE"
    fi
    echo "----------------------------------------"
done

echo ">>> All Tests Finished <<<"
