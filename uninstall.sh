#!/bin/bash

# ============================================
# TejX Toolchain - Uninstallation Script
# ============================================

TEJX_HOME="$HOME/.tejx"

echo ">>> Uninstalling TejX Toolchain from $TEJX_HOME..."
rm -rf "$TEJX_HOME"

echo ">>> Removing from PATH..."
for config in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile"; do
    if [ -f "$config" ]; then
        # Remove the lines added by install.sh (using a robust pattern)
        sed -i '' '/# TejX Toolchain/d' "$config" 2>/dev/null || sed -i '/# TejX Toolchain/d' "$config"
        sed -i '' '/\.tejx\/bin/d' "$config" 2>/dev/null || sed -i '/\.tejx\/bin/d' "$config"
    fi
done

echo "✅ TejX Toolchain uninstalled."
echo "   Please restart your terminal to apply PATH changes."
