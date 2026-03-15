#!/bin/bash

# ============================================
# TejX Compiler - Build Script
# ============================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo ">>> Building TejX Compiler..."

# Build in release mode
cargo build --release 2>&1

if [ $? -eq 0 ]; then
    echo "✅ Compiler Build successful!"
    echo "   Binary:  $SCRIPT_DIR/target/release/tejxc"

    # Find the runtime library (it might have a hash in the name like libtejx_rt-xxxx.a)
    PROFILE_DIR="$SCRIPT_DIR/target/release"
    [ ! -d "$PROFILE_DIR" ] && PROFILE_DIR="$SCRIPT_DIR/target/debug"

    # Find libtejx_rt*.a in deps/ and copy it to a predictable location
    RT_FILE=$(find "$PROFILE_DIR/deps" -name "libtejx_rt*.a" | head -n 1)
    if [ -n "$RT_FILE" ]; then
        cp "$RT_FILE" "$PROFILE_DIR/tejx_rt.a"
        echo "   Runtime: $PROFILE_DIR/tejx_rt.a"
    else
        echo "   ⚠️ Warning: Runtime library (libtejx_rt.a) not found in $PROFILE_DIR/deps"
    fi
else
    echo "❌ Compiler Build failed."
    exit 1
fi
