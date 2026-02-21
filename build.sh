#!/bin/bash

# ============================================
# TejX Rust Compiler - Build Script
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo ">>> Building TejX Compiler..."

# Build in release mode
cargo build --release 2>&1

if [ $? -eq 0 ]; then
    echo "✅ Compiler Build successful!"
    echo "   Binary: $SCRIPT_DIR/target/release/tejxc"
else
    echo "❌ Compiler Build failed."
    exit 1
fi
