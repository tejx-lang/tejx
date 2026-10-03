#!/bin/bash

# ============================================
# TejX Toolchain - Installation Script
# ============================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEJX_HOME="${TEJX_HOME:-$HOME/.tejx}"

echo ">>> Building TejX Toolchain..."
./build.sh

echo ">>> Installing to $TEJX_HOME..."
mkdir -p "$TEJX_HOME/bin" "$TEJX_HOME/lib" "$TEJX_HOME/runtime"

cp "$SCRIPT_DIR/target/release/tejxc" "$TEJX_HOME/bin/"
chmod +x "$TEJX_HOME/bin/tejxc"

if [ -f "$SCRIPT_DIR/target/release/tejx_rt.a" ]; then
    cp "$SCRIPT_DIR/target/release/tejx_rt.a" "$TEJX_HOME/runtime/tejx_rt.a"
elif [ -f "$SCRIPT_DIR/target/release/libtejx_rt.a" ]; then
    cp "$SCRIPT_DIR/target/release/libtejx_rt.a" "$TEJX_HOME/runtime/tejx_rt.a"
else
    echo "❌ Error: Runtime library (tejx_rt.a or libtejx_rt.a) not found in $SCRIPT_DIR/target/release"
    exit 1
fi

cp -R "$SCRIPT_DIR/src/library/"* "$TEJX_HOME/lib/"

# Link extension if it exists
EXT_DIR="$SCRIPT_DIR/editors/antigravity"
if [ ! -d "$EXT_DIR" ] && [ -d "$SCRIPT_DIR/../vs-code-extention" ]; then
    EXT_DIR="$SCRIPT_DIR/../vs-code-extention"
fi

if [ -d "$EXT_DIR" ]; then
    echo ">>> Linking VS Code extension..."
    mkdir -p "$HOME/.vscode/extensions"
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
        case "$SHELL" in
            */zsh)
                shell_config="$HOME/.zshrc"
                ;;
            */bash)
                case "$(uname -s)" in
                    Darwin) shell_config="$HOME/.bash_profile" ;;
                    *)      shell_config="$HOME/.bashrc" ;;
                esac
                ;;
            *)
                if [ -f "$HOME/.zshrc" ]; then shell_config="$HOME/.zshrc"
                elif [ -f "$HOME/.bash_profile" ]; then shell_config="$HOME/.bash_profile"
                elif [ -f "$HOME/.bashrc" ]; then shell_config="$HOME/.bashrc"
                else shell_config="$HOME/.profile"
                fi
                ;;
        esac
        
        [ -z "$shell_config" ] && shell_config="$HOME/.profile"
        touch "$shell_config"
        
        if ! grep -q "\.tejx/bin" "$shell_config"; then
            echo "" >> "$shell_config"
            echo "# TejX Toolchain" >> "$shell_config"
            echo "export PATH=\"\$HOME/.tejx/bin:\$PATH\"" >> "$shell_config"
            echo ">>> Added TejX to PATH in $shell_config"
            echo ">>> Please restart your terminal or run: source $shell_config"
        else
            echo ">>> TejX is already in PATH configuration ($shell_config)"
        fi
        ;;
esac
