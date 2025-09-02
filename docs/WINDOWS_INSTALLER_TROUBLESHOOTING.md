# Windows Installer Troubleshooting

## Common Issues and Solutions

### Installer asks for admin permissions then does nothing

**Problem**: The MSI installer requests administrator privileges but then appears to complete without showing any progress or confirmation.

**Cause**: The installer was running in silent mode without a user interface.

**Solution**: This has been fixed in the latest builds. The installer now includes:
- A progress dialog showing installation status
- A completion screen with instructions about PATH changes

### Testing the Installer Locally

If you're experiencing issues, you can test with a UI-enabled installer:

1. Run the test script:
   ```powershell
   .\test-windows-installer-with-ui.ps1
   ```

2. This creates an installer with full UI including:
   - Welcome screen
   - Installation directory selection (WixUI_InstallDir version)
   - Progress bar
   - Completion message

### Manual Installation Steps

If the installer continues to have issues:

1. **Extract manually**:
   ```powershell
   msiexec /a digstore-windows-x64.msi /qb TARGETDIR=C:\temp\digstore
   ```

2. **Copy files**:
   ```powershell
   xcopy C:\temp\digstore\Digstore "C:\Program Files\Digstore" /E /I
   ```

3. **Add to PATH manually**:
   - Open System Properties → Environment Variables
   - Add `C:\Program Files\Digstore` to PATH
   - Click OK and restart your terminal

### Verifying Installation

After installation:

```powershell
# Check if digstore is in PATH
where digstore

# Test the executable directly
"C:\Program Files\Digstore\digstore.exe" --version

# Check registry entries
reg query HKCU\Software\Digstore

# Check environment variables
echo $env:PATH | Select-String -Pattern "Digstore"
```

### Uninstalling

To remove Digstore:

1. **Via Control Panel**: 
   - Apps & Features → Digstore Min → Uninstall

2. **Via Command Line**:
   ```powershell
   msiexec /x digstore-windows-x64.msi
   ```

3. **Manual cleanup** (if needed):
   ```powershell
   Remove-Item -Recurse -Force "C:\Program Files\Digstore"
   reg delete HKCU\Software\Digstore /f
   ```

### Installation Logs

To debug installation issues, enable logging:

```powershell
msiexec /i digstore-windows-x64.msi /l*v install.log
```

Then check `install.log` for errors.

### Known Issues

1. **PATH not updated immediately**: Windows requires a terminal restart or logout/login for PATH changes to take effect.

2. **Antivirus interference**: Some antivirus software may block the installer. Temporarily disable it during installation.

3. **Missing Visual C++ Runtime**: If you get dll errors, install the Visual C++ Redistributable from Microsoft.

### Getting Help

If you continue to experience issues:

1. Check the [installation log](#installation-logs)
2. Open an issue on [GitHub](https://github.com/DIG-Network/digstore/issues)
3. Include:
   - Windows version (`winver`)
   - Error messages or screenshots
   - Installation log file
