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
    FILENAME="digstore-macos.dmg"
    INSTALL_BASE="/usr/local/lib/digstore"
else
    PLATFORM="linux"
    FILENAME="digstore-linux-x86_64.AppImage"
    INSTALL_BASE="/usr/local/lib/digstore"
fi

VERSION_DIR="$INSTALL_BASE/v$VERSION"

echo "Platform: $PLATFORM"
echo "Installing to: $VERSION_DIR"

# Check if already installed
if [[ -d "$VERSION_DIR" && "$FORCE" != "true" ]]; then
    echo "Version $VERSION is already installed at: $VERSION_DIR"
    echo "Use 'force' as second argument to reinstall"
    exit 0
fi

# Check for sudo access
if [[ ! -w "/usr/local" ]]; then
    echo "This script requires sudo privileges to install to /usr/local"
    echo "You may be prompted for your password..."
fi

# Create directories
echo "Creating installation directory..."
sudo mkdir -p "$VERSION_DIR"

# Download the installer
if [[ "$VERSION" == "latest" ]]; then
    DOWNLOAD_URL="https://github.com/DIG-Network/digstore/releases/latest/download/$FILENAME"
else
    DOWNLOAD_URL="https://github.com/DIG-Network/digstore/releases/download/v$VERSION/$FILENAME"
fi

TEMP_INSTALLER="/tmp/digstore-installer.$PLATFORM"

echo "Downloading from: $DOWNLOAD_URL"
if command -v curl >/dev/null 2>&1; then
    curl -L -o "$TEMP_INSTALLER" "$DOWNLOAD_URL" --user-agent "DigStore-Bootstrap"
elif command -v wget >/dev/null 2>&1; then
    wget -O "$TEMP_INSTALLER" "$DOWNLOAD_URL" --user-agent="DigStore-Bootstrap"
else
    echo "❌ Neither curl nor wget found. Please install one of them."
    exit 1
fi

echo "✅ Download completed"

# Extract and install based on platform
if [[ "$PLATFORM" == "macos" ]]; then
    echo "Extracting DMG..."
    
    # Mount DMG
    MOUNT_OUTPUT=$(hdiutil attach "$TEMP_INSTALLER" -nobrowse)
    MOUNT_POINT=$(echo "$MOUNT_OUTPUT" | grep -E '/Volumes/' | awk '{print $NF}')
    
    if [[ -z "$MOUNT_POINT" ]]; then
        echo "❌ Failed to mount DMG"
        rm -f "$TEMP_INSTALLER"
        exit 1
    fi
    
    # Copy binary from app bundle
    APP_PATH="$MOUNT_POINT/DIG Network Digstore.app"
    BINARY_SOURCE="$APP_PATH/Contents/MacOS/digstore"
    BINARY_TARGET="$VERSION_DIR/digstore"
    
    if [[ -f "$BINARY_SOURCE" ]]; then
        sudo cp "$BINARY_SOURCE" "$BINARY_TARGET"
        sudo chmod +x "$BINARY_TARGET"
        echo "✅ Binary extracted to versioned directory"
    else
        echo "❌ Could not find digstore binary in DMG"
        hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1
        rm -f "$TEMP_INSTALLER"
        exit 1
    fi
    
    # Unmount DMG
    hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1
    
elif [[ "$PLATFORM" == "linux" ]]; then
    echo "Installing AppImage..."
    
    # Copy AppImage directly to versioned directory
    BINARY_TARGET="$VERSION_DIR/digstore"
    sudo cp "$TEMP_INSTALLER" "$BINARY_TARGET"
    sudo chmod +x "$BINARY_TARGET"
    
    echo "✅ AppImage installed to versioned directory"
fi

# Update PATH
echo "Updating PATH..."
SHELL_RC=""
if [[ -n "$ZSH_VERSION" ]]; then
    SHELL_RC="$HOME/.zshrc"
elif [[ -n "$BASH_VERSION" ]]; then
    SHELL_RC="$HOME/.bashrc"
fi

if [[ -n "$SHELL_RC" ]]; then
    # Remove existing dig-network PATH entries
    grep -v "dig-network" "$SHELL_RC" > "${SHELL_RC}.tmp" 2>/dev/null || touch "${SHELL_RC}.tmp"
    
    # Add new PATH entry
    echo "export PATH=\"$VERSION_DIR:\$PATH\"" >> "${SHELL_RC}.tmp"
    mv "${SHELL_RC}.tmp" "$SHELL_RC"
    
    echo "✅ Updated $SHELL_RC"
else
    echo "⚠️ Could not detect shell. Please manually add to your PATH:"
    echo "export PATH=\"$VERSION_DIR:\$PATH\""
fi

# Save active version
sudo sh -c "echo '$VERSION' > '$INSTALL_BASE/active'"

# Clean up
rm -f "$TEMP_INSTALLER"

echo ""
echo "🎉 Digstore $VERSION installed successfully!"
echo "Location: $VERSION_DIR"
echo ""
echo "Usage:"
echo "  digstore --version                    # Check installed version"
echo "  digstore init                         # Initialize a repository"
echo "  digstore version list                 # List installed versions"
echo "  digstore version install-version 0.4.8  # Install specific version"
echo "  digstore version set 0.4.8           # Switch to version"
echo "  digstore update                       # Update to latest"
echo ""
echo "For help: digstore --help"
echo ""
echo "⚠️ Restart your terminal or run: source $SHELL_RC"
