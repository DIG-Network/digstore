# Digstore Installation

## Quick Install (Recommended)

### Windows
```powershell
# Run in PowerShell as Administrator
iex (iwr -Uri "https://raw.githubusercontent.com/DIG-Network/digstore/main/install.ps1").Content
```

### Linux/macOS
```bash
# Run in terminal
curl -fsSL https://raw.githubusercontent.com/DIG-Network/digstore/main/install.sh | bash
```

## What the Bootstrap Installer Does

1. **Downloads** the latest digstore binary for your platform
2. **Installs** to versioned directory structure:
   - Windows: `C:\Program Files (x86)\dig-network\v0.4.7\digstore.exe`
   - macOS: `/usr/local/lib/digstore/v0.4.7/digstore`
   - Linux: `/usr/local/lib/digstore/v0.4.7/digstore`
3. **Updates PATH** to point to the installed version
4. **Sets up version management** for future updates

## After Installation

Once installed, you can manage versions like nvm:

```bash
# Check current version
digstore --version

# List installed versions  
digstore version list

# Install specific version
digstore version install-version 0.4.8

# Switch between versions
digstore version set 0.4.7
digstore version set 0.4.8

# Update to latest
digstore update

# Fix PATH conflicts
digstore version fix-path-auto
```

## Manual Installation

If you prefer manual installation:

### Windows
1. Download [digstore-windows-x64.msi](https://github.com/DIG-Network/digstore/releases/latest/download/digstore-windows-x64.msi)
2. Run: `digstore version install-msi digstore-windows-x64.msi`
3. Run: `digstore version fix-path-auto`

### macOS
1. Download [digstore-macos.dmg](https://github.com/DIG-Network/digstore/releases/latest/download/digstore-macos.dmg)
2. Run: `digstore version install-msi digstore-macos.dmg` 
3. Run: `digstore version fix-path-auto`

### Linux
1. Download [digstore-linux-x86_64.AppImage](https://github.com/DIG-Network/digstore/releases/latest/download/digstore-linux-x86_64.AppImage)
2. Run: `digstore version install-msi digstore-linux-x86_64.AppImage`
3. Run: `digstore version fix-path-auto`

## Build from Source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/DIG-Network/digstore.git
cd digstore
cargo build --release

# Install with version management
./target/release/digstore version install-current
./target/release/digstore version fix-path-auto
```

## Troubleshooting

### Permission Issues
- **Windows**: Run PowerShell as Administrator
- **Linux/macOS**: Ensure you have sudo access

### PATH Issues
```bash
digstore version fix-path          # Analyze PATH conflicts
digstore version fix-path-auto     # Automatically fix PATH
```

### Version Conflicts
```bash
digstore version list              # See installed versions
digstore version set <version>     # Switch to specific version
digstore version remove <version>  # Remove old versions
```
