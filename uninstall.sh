#!/bin/bash

# ============================================
# TejX Toolchain - Uninstallation Script
# ============================================

set -e

TEJX_HOME="${TEJX_HOME:-$HOME/.tejx}"

echo ">>> Uninstalling TejX Toolchain from $TEJX_HOME..."
if [ -d "$TEJX_HOME" ]; then
    rm -rf "$TEJX_HOME"
    echo ">>> Removed $TEJX_HOME"
else
    echo ">>> $TEJX_HOME not found, skipping."
fi

echo ">>> Removing extension links..."
rm -f "$HOME/.vscode/extensions/tejx-antigravity" 2>/dev/null || true
rm -f "$HOME/.antigravity/extensions/tejx-antigravity" 2>/dev/null || true

echo ">>> Removing from PATH..."
for config in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile"; do
    if [ -f "$config" ]; then
        if grep -q "\.tejx/bin" "$config" 2>/dev/null || grep -q "# TejX Toolchain" "$config" 2>/dev/null; then
            TMP_CONFIG="$(mktemp)"
            grep -v "# TejX Toolchain" "$config" | grep -v "\.tejx/bin" > "$TMP_CONFIG" && cat "$TMP_CONFIG" > "$config"
            rm -f "$TMP_CONFIG"
            echo ">>> Cleaned PATH from $config"
        fi
    fi
done

echo "✅ TejX Toolchain uninstalled."
echo "   Please restart your terminal to apply PATH changes."
