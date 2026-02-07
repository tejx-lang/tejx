#!/bin/bash

# Create build directory if it doesn't exist
mkdir -p build

# Compile the compiler (tejxc)
echo "Compiling TejX Compiler (tejxc)..."
SOURCES="src/main.cpp src/lexer/Lexer.cpp src/parser/Parser.cpp src/codegen/CodeGen.cpp"

clang++ -std=c++17 -Iinclude $SOURCES -o build/tejxc

if [ $? -eq 0 ]; then
    echo "Compiler Build successful! Binary located at: build/tejxc"
else
    echo "Compiler Build failed."
    exit 1
fi
