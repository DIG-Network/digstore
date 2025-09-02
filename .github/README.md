# GitHub Actions Workflows

This directory contains GitHub Actions workflows for building, testing, and releasing Digstore Min.

## Workflows

### 1. CI (`ci.yml`)
**Trigger**: On push to main/master and pull requests

This workflow runs on every push and pull request to ensure code quality:
- **Tests**: Runs on Ubuntu, Windows, and macOS with stable and beta Rust
- **Linting**: Checks formatting with `cargo fmt` and runs `cargo clippy`
- **Security**: Runs `cargo audit` for vulnerability scanning
- **Coverage**: Generates code coverage reports with `cargo-llvm-cov`
- **Documentation**: Builds and deploys documentation to GitHub Pages
- **Binary Artifacts**: Creates release binaries for all platforms

### 2. Build Installers (`build-installers.yml`)
**Trigger**: On push to main/master, pull requests, and manual dispatch

This workflow builds proper installers for all platforms:
- **Windows**: Creates MSI installer using WiX Toolset
  - Installs to Program Files
  - Adds to system PATH automatically
  - Creates Start Menu shortcuts
- **macOS**: Creates DMG installer
  - Builds universal binary (Intel + Apple Silicon)
  - Drag-and-drop installation
- **Linux**: Creates multiple package formats
  - DEB package for Debian/Ubuntu
  - RPM package for Fedora/RHEL
  - AppImage for universal Linux support
- **Install Script**: Universal installation script that detects the platform

### 3. Release (`release.yml`)
**Trigger**: On version tags (e.g., `v1.0.0`)

This workflow creates official releases:
- **GitHub Release**: Creates release with changelog
- **Binaries**: Builds optimized binaries for all platforms
- **Installers**: Creates platform-specific installers
- **Crates.io**: Publishes to Rust package registry
- **Homebrew Formula**: Generates formula for macOS users
- **Installation Script**: Creates universal install script

## Platform Support

### Binaries
- Linux x86_64
- Linux ARM64
- macOS x86_64 (Intel)
- macOS ARM64 (Apple Silicon)
- Windows x86_64

### Installers
- **Windows**: MSI installer
- **macOS**: DMG with universal binary
- **Linux**: DEB, RPM, and AppImage

## Artifacts

All workflows produce downloadable artifacts:
- **Binaries**: Raw executables for each platform
- **Installers**: Platform-specific installer packages
- **Scripts**: Installation and completion scripts

## Usage

### For Contributors
1. Push changes to a branch
2. CI workflow runs automatically
3. Check workflow status before merging

### For Releases
1. Update version in `Cargo.toml`
2. Create and push a version tag: `git tag v1.0.0 && git push --tags`
3. Release workflow creates installers and publishes automatically

### Manual Builds
You can manually trigger the installer build workflow:
1. Go to Actions tab
2. Select "Build Installers"
3. Click "Run workflow"

## Secrets Required

The following secrets need to be configured in repository settings:
- `CRATES_TOKEN`: API token for publishing to crates.io (optional)
- `GITHUB_TOKEN`: Automatically provided by GitHub Actions

## Maintenance

To update installer configurations:
- **Windows**: Modify the WiX XML in the workflow
- **macOS**: Update the DMG creation process
- **Linux**: Adjust FPM parameters or AppImage configuration
