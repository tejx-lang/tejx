#!/bin/bash

# ============================================
# TejX Compiler - Build Script
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo ">>> Building TejX Compiler..."

# Build workspace in release mode (compiler + runtime staticlib)
cargo build --release --workspace 2>&1

if [ $? -eq 0 ]; then
    echo "✅ Compiler Build successful!"
    echo "   Binary:  $SCRIPT_DIR/target/release/tejxc"

    PROFILE_DIR="$SCRIPT_DIR/target/release"
    [ ! -d "$PROFILE_DIR" ] && PROFILE_DIR="$SCRIPT_DIR/target/debug"

    if [ -f "$PROFILE_DIR/libtejx_rt.a" ]; then
        cp "$PROFILE_DIR/libtejx_rt.a" "$PROFILE_DIR/tejx_rt.a"
        echo "   Runtime: $PROFILE_DIR/tejx_rt.a"
    else
        echo "   ⚠️ Warning: Runtime library (libtejx_rt.a) not found in $PROFILE_DIR"
    fi
else
    echo "❌ Compiler Build failed."
    exit 1
fi
