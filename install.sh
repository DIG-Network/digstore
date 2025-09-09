#!/bin/bash
# DigStore Bootstrap Installer for Linux/macOS
# This script downloads and installs the latest version of DigStore with version management

set -e

VERSION="${1:-latest}"
FORCE="${2:-false}"

echo "🚀 DigStore Bootstrap Installer"
echo "=================================="
echo ""

# Determine platform and installation directory
if [[ "$OSTYPE" == "darwin"* ]]; then
    PLATFORM="macos"
    BINARY_NAME="digstore-macos-universal"
    SYSTEM_INSTALL_DIR="/usr/local/lib/digstore"
else
    PLATFORM="linux"
    BINARY_NAME="digstore-linux-x64"
    SYSTEM_INSTALL_DIR="/usr/local/lib/digstore"
fi

USER_INSTALL_DIR="$HOME/.digstore-versions"

# Try system installation first, fallback to user directory
INSTALL_DIR="$SYSTEM_INSTALL_DIR"
IS_SYSTEM_INSTALL=true

# Test write access to system directory
if ! mkdir -p "$SYSTEM_INSTALL_DIR" 2>/dev/null || ! touch "$SYSTEM_INSTALL_DIR/.test" 2>/dev/null; then
    echo "⚠️  Cannot write to system directory, using user directory: $USER_INSTALL_DIR"
    INSTALL_DIR="$USER_INSTALL_DIR"
    IS_SYSTEM_INSTALL=false
    mkdir -p "$INSTALL_DIR"
else
    echo "✅ System installation directory accessible: $SYSTEM_INSTALL_DIR"
    rm -f "$SYSTEM_INSTALL_DIR/.test"
fi

# Fetch latest release information
API_URL="https://api.github.com/repos/DIG-Network/digstore/releases/latest"
echo "Fetching release information..."

if ! RELEASE_INFO=$(curl -s "$API_URL"); then
    echo "❌ Failed to fetch release information"
    exit 1
fi

if [ "$VERSION" = "latest" ]; then
    VERSION=$(echo "$RELEASE_INFO" | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/')
    echo "Latest version: $VERSION"
fi

# Construct download URL
DOWNLOAD_URL="https://github.com/DIG-Network/digstore/releases/download/v$VERSION/$BINARY_NAME-v$VERSION"

echo "Download URL: $DOWNLOAD_URL"

# Download binary
TEMP_BINARY="/tmp/digstore-$VERSION"

echo "Downloading binary..."
if command -v curl >/dev/null 2>&1; then
    curl -L -o "$TEMP_BINARY" "$DOWNLOAD_URL" --user-agent "DigStore-Bootstrap"
elif command -v wget >/dev/null 2>&1; then
    wget -O "$TEMP_BINARY" "$DOWNLOAD_URL" --user-agent="DigStore-Bootstrap"
else
    echo "❌ Neither curl nor wget found. Please install one of them."
    exit 1
fi

if [ ! -f "$TEMP_BINARY" ]; then
    echo "❌ Download failed"
    exit 1
fi

echo "✅ Download completed"

# Create version directory and install binary
VERSION_DIR="$INSTALL_DIR/v$VERSION"
echo "Installing to: $VERSION_DIR"

mkdir -p "$VERSION_DIR"
cp "$TEMP_BINARY" "$VERSION_DIR/digstore"
chmod +x "$VERSION_DIR/digstore"

echo "✅ Binary installed successfully"

# Update PATH
echo "Updating PATH..."

if [ "$IS_SYSTEM_INSTALL" = true ]; then
    # Create symlink in /usr/local/bin for system-wide access
    SYMLINK_PATH="/usr/local/bin/digstore"
    if ln -sf "$VERSION_DIR/digstore" "$SYMLINK_PATH" 2>/dev/null; then
        echo "✅ System symlink created: $SYMLINK_PATH"
    else
        echo "⚠️  Could not create system symlink, you may need to add $VERSION_DIR to your PATH"
    fi
else
    # Update user's shell profile
    SHELL_PROFILE=""
    if [ -n "$BASH_VERSION" ]; then
        SHELL_PROFILE="$HOME/.bashrc"
    elif [ -n "$ZSH_VERSION" ]; then
        SHELL_PROFILE="$HOME/.zshrc"
    elif [ -f "$HOME/.profile" ]; then
        SHELL_PROFILE="$HOME/.profile"
    fi
    
    if [ -n "$SHELL_PROFILE" ]; then
        if ! grep -q "digstore-versions" "$SHELL_PROFILE" 2>/dev/null; then
            echo "" >> "$SHELL_PROFILE"
            echo "# DigStore version manager" >> "$SHELL_PROFILE"
            echo "export PATH=\"$VERSION_DIR:\$PATH\"" >> "$SHELL_PROFILE"
            echo "✅ Added to $SHELL_PROFILE"
        else
            echo "ℹ️  PATH already configured in $SHELL_PROFILE"
        fi
    fi
    
    # Update current session
    export PATH="$VERSION_DIR:$PATH"
    echo "✅ Current session PATH updated"
fi

# Test installation
echo "Testing installation..."
if "$VERSION_DIR/digstore" --version >/dev/null 2>&1; then
    TEST_RESULT=$("$VERSION_DIR/digstore" --version)
    echo "✅ Installation test successful: $TEST_RESULT"
else
    echo "⚠️  Installation test failed"
fi

# Clean up
rm -f "$TEMP_BINARY"

echo ""
echo "🎉 DigStore $VERSION installed successfully!"
echo "Location: $VERSION_DIR"
echo ""
echo "Usage:"
echo "  digstore --version                    # Check installed version"
echo "  digstore init                         # Initialize a repository"
echo "  digstore version list                 # List installed versions" 
echo "  digstore version set <version>        # Switch to a different version"
echo ""

if [ "$IS_SYSTEM_INSTALL" = false ]; then
    echo "⚠️  Note: Installed to user directory. You may need to restart your terminal"
    echo "   or run 'source ~/.bashrc' (or ~/.zshrc) for PATH changes to take effect."
else
    echo "ℹ️  Note: System installation complete. The 'digstore' command should be available immediately."
fi

echo ""
echo "📚 Documentation: https://github.com/DIG-Network/digstore"