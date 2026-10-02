#!/bin/bash

# ============================================
# TejX Toolchain - Installation Script
# ============================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEJX_HOME="$HOME/.tejx"

echo ">>> Building TejX Toolchain..."
./build.sh

echo ">>> Installing to $TEJX_HOME..."
mkdir -p "$TEJX_HOME/bin" "$TEJX_HOME/lib" "$TEJX_HOME/runtime"

cp "$SCRIPT_DIR/target/release/tejxc" "$TEJX_HOME/bin/"
cp "$SCRIPT_DIR/target/release/tejx_rt.a" "$TEJX_HOME/runtime/"
cp -R "$SCRIPT_DIR/src/library/"* "$TEJX_HOME/lib/"

# Link extension if it exists
EXT_DIR="$SCRIPT_DIR/editors/antigravity"
if [ -d "$EXT_DIR" ]; then
    echo ">>> Linking VS Code extension..."
    ln -sf "$EXT_DIR" "$HOME/.vscode/extensions/tejx-antigravity"
    
    if [ -d "$HOME/.antigravity/extensions" ]; then
        echo ">>> Linking Antigravity extension..."
        ln -sf "$EXT_DIR" "$HOME/.antigravity/extensions/tejx-antigravity"
    fi
fi

# Add to PATH if not already present
case ":$PATH:" in
    *":$TEJX_HOME/bin:"*) 
        echo ">>> TejX is already in your PATH."
        ;;
    *)
        shell_config=""
        if [[ "$SHELL" == */zsh ]]; then
            shell_config="$HOME/.zshrc"
        elif [[ "$SHELL" == */bash ]]; then
            if [[ "$OSTYPE" == "darwin"* ]]; then
                shell_config="$HOME/.bash_profile"
            else
                shell_config="$HOME/.bashrc"
            fi
        fi
        
        [ -z "$shell_config" ] && shell_config="$HOME/.profile"
        
        if [ -f "$shell_config" ]; then
            if ! grep -q "\.tejx/bin" "$shell_config"; then
                echo "" >> "$shell_config"
                echo "# TejX Toolchain" >> "$shell_config"
                echo "export PATH=\"\$HOME/.tejx/bin:\$PATH\"" >> "$shell_config"
                echo ">>> Added TejX to PATH in $shell_config"
                echo ">>> Please restart your terminal or run: source $shell_config"
            else
                echo ">>> TejX is already in PATH configuration ($shell_config)"
            fi
        else
            echo ">>> Please add $TEJX_HOME/bin to your PATH manually."
        fi
        ;;
esac
