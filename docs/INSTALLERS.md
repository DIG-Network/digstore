# Digstore Installers

This document explains how to download and use Digstore installers.

## Download Options

### 1. Latest Development Build (Recommended for Testing)

The latest development build is automatically updated after each successful build on the main branch:

🔗 **[Download Latest Build](https://github.com/DIG-Network/digstore/releases/tag/latest-build)**

This includes:
- Windows MSI installer
- macOS DMG installer
- Linux packages (DEB, RPM, AppImage)
- Universal install script

### 2. Stable Releases

For production use, download from the official releases page:

🔗 **[All Releases](https://github.com/DIG-Network/digstore/releases)**

### 3. Workflow Artifacts

Recent builds are also available as workflow artifacts (30-day retention):

1. Go to [Actions](https://github.com/DIG-Network/digstore/actions)
2. Click on a recent "Build Installers" workflow run
3. Scroll down to "Artifacts" section
4. Download the installer for your platform

## Installation Instructions

### Windows

1. Download `digstore-windows-x64.msi`
2. Run the installer (requires Administrator privileges)
3. The installer will:
   - Install to Program Files
   - Add digstore to system PATH
   - Create Start Menu shortcuts
4. Restart your terminal or log out/in for PATH changes to take effect

### macOS

1. Download `digstore-macos.dmg`
2. Open the DMG file
3. Drag "Digstore Min" to your Applications folder
4. Add to PATH (optional):
   ```bash
   echo 'export PATH="/Applications/Digstore Min.app/Contents/MacOS:$PATH"' >> ~/.zshrc
   source ~/.zshrc
   ```

### Linux

#### Debian/Ubuntu (.deb)
```bash
sudo dpkg -i digstore_0.1.0_amd64.deb
```

#### Fedora/RHEL (.rpm)
```bash
sudo rpm -i digstore-0.1.0-1.x86_64.rpm
```

#### Universal (AppImage)
```bash
chmod +x digstore-linux-x86_64.AppImage
./digstore-linux-x86_64.AppImage
```

### Universal Install Script

For automated installation on any platform:

```bash
curl -L https://github.com/DIG-Network/digstore/releases/download/latest-build/install.sh | bash
```

## Verifying Installation

After installation, verify digstore is working:

```bash
digstore --version
digstore --help
```

## Uninstalling

### Windows
Use "Add or Remove Programs" in Windows Settings

### macOS
1. Delete from Applications folder
2. Remove from PATH if added

### Linux
- Debian/Ubuntu: `sudo apt remove digstore`
- Fedora/RHEL: `sudo rpm -e digstore`
- AppImage: Just delete the file

## Automation

### CI/CD Integration

To download the latest installer in your CI/CD pipeline:

```bash
# Windows
curl -L https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-windows-x64.msi -o digstore.msi

# macOS
curl -L https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-macos.dmg -o digstore.dmg

# Linux
curl -L https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-linux-x86_64.AppImage -o digstore
chmod +x digstore
```

### PowerShell (Windows)
```powershell
Invoke-WebRequest -Uri "https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-windows-x64.msi" -OutFile "digstore.msi"
Start-Process msiexec.exe -ArgumentList '/i', 'digstore.msi', '/quiet' -Wait
```

## Build Your Own

To build installers from source:

```bash
git clone https://github.com/DIG-Network/digstore.git
cd digstore

# Build release binary
cargo build --release

# Run installer build workflow locally (requires act)
act -j build-windows-installer
act -j build-macos-installer
act -j build-linux-packages
```

## Troubleshooting

### Windows PATH not updated
- Log out and back in, or restart your computer
- Manually add `C:\Program Files\Digstore` to PATH

### macOS "unidentified developer" warning
- Right-click the app and select "Open"
- Or: System Preferences → Security & Privacy → Allow

### Linux permission denied
- Ensure the binary has execute permissions: `chmod +x digstore`
- Install to a directory in PATH: `sudo mv digstore /usr/local/bin/`

## Support

For issues with installers:
- [Open an issue](https://github.com/DIG-Network/digstore/issues/new)
- Include your OS version and any error messages
