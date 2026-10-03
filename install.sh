#!/bin/bash

# ============================================
# TejX Toolchain - Installation Script
# ============================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEJX_HOME="${TEJX_HOME:-$HOME/.tejx}"

# ── Ensure System Dependencies ──
OS="$(uname -s)"
echo ">>> Verifying system requirements (Clang, build-essential, OpenSSL)..."

clang_bin=""
for c in clang clang-19 clang-18 clang-17 clang-16 clang-15; do
    if command -v "$c" >/dev/null 2>&1; then
        clang_bin="$c"
        break
    fi
done

need_deps=0
if [ -z "$clang_bin" ]; then
    need_deps=1
fi

if [ "$OS" = "Linux" ]; then
    ssl_found=0
    for dir in /usr/lib /usr/lib64 /usr/local/lib /usr/lib/*-linux-* /lib/*-linux-*; do
        if [ -f "$dir/libssl.so" ] || [ -f "$dir/libssl.so.3" ] || [ -f "$dir/libssl.so.1.1" ]; then
            ssl_found=1
            break
        fi
    done
    if [ "$ssl_found" -eq 0 ]; then
        need_deps=1
    fi

    if ! command -v make >/dev/null 2>&1 || ! command -v ld >/dev/null 2>&1; then
        need_deps=1
    fi

    if [ "$need_deps" -eq 1 ]; then
        echo ">>> Installing required build dependencies (Clang, build-essential, OpenSSL)..."
        sudo_cmd=""
        if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
            sudo_cmd="sudo"
        fi

        if command -v apt-get >/dev/null 2>&1; then
            deps="clang build-essential libssl-dev"
            echo ">>> Installing $deps via apt-get..."
            if [ -n "$sudo_cmd" ]; then
                $sudo_cmd apt-get update -qq && $sudo_cmd apt-get install -y -qq $deps || echo "⚠️ Warning: Please run: sudo apt-get install -y $deps"
            elif [ "$(id -u)" -eq 0 ]; then
                apt-get update -qq && apt-get install -y -qq $deps || echo "⚠️ Warning: Please run: apt-get install -y $deps"
            fi
        elif command -v dnf >/dev/null 2>&1; then
            deps="clang gcc openssl-devel"
            echo ">>> Installing $deps via dnf..."
            if [ -n "$sudo_cmd" ]; then
                $sudo_cmd dnf install -y -q $deps || echo "⚠️ Warning: Please run: sudo dnf install -y $deps"
            elif [ "$(id -u)" -eq 0 ]; then
                dnf install -y -q $deps || echo "⚠️ Warning: Please run: dnf install -y $deps"
            fi
        elif command -v pacman >/dev/null 2>&1; then
            deps="clang base-devel openssl"
            echo ">>> Installing $deps via pacman..."
            if [ -n "$sudo_cmd" ]; then
                $sudo_cmd pacman -Sy --noconfirm --needed $deps || echo "⚠️ Warning: Please run: sudo pacman -S $deps"
            elif [ "$(id -u)" -eq 0 ]; then
                pacman -Sy --noconfirm --needed $deps || echo "⚠️ Warning: Please run: pacman -S $deps"
            fi
        elif command -v apk >/dev/null 2>&1; then
            deps="clang build-base openssl-dev"
            echo ">>> Installing $deps via apk..."
            if [ -n "$sudo_cmd" ]; then
                $sudo_cmd apk add --no-cache $deps || echo "⚠️ Warning: Please run: sudo apk add $deps"
            elif [ "$(id -u)" -eq 0 ]; then
                apk add --no-cache $deps || echo "⚠️ Warning: Please run: apk add $deps"
            fi
        fi
    fi
elif [ "$OS" = "Darwin" ]; then
    if [ -z "$clang_bin" ]; then
        echo ">>> Clang not found. Triggering Xcode Command Line Tools install..."
        xcode-select --install 2>/dev/null || true
    fi
fi

# Post-check Clang
for c in clang clang-19 clang-18 clang-17 clang-16 clang-15; do
    if command -v "$c" >/dev/null 2>&1; then
        clang_bin="$c"
        break
    fi
done

if [ -n "$clang_bin" ]; then
    clang_ver="$($clang_bin --version 2>/dev/null | head -n 1)"
    echo ">>> Compiler backend verified: $clang_ver"
else
    echo "⚠️ Warning: Clang compiler not detected. Please ensure clang is installed for native builds."
fi

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
