# Digstore Bootstrap Installer
# This script downloads and installs the latest version of Digstore with version management

param(
    [string]$Version = "latest",
    [switch]$Force = $false
)

$ErrorActionPreference = "Stop"

Write-Host "🚀 Digstore Bootstrap Installer" -ForegroundColor Cyan
Write-Host "=================================" -ForegroundColor Cyan
Write-Host ""

# Determine installation directory
$ProgramFiles = $env:ProgramFiles
if ($env:PROCESSOR_ARCHITECTURE -eq "AMD64" -and $env:ProgramW6432) {
    $ProgramFiles = ${env:ProgramFiles(x86)}
}
$InstallBase = Join-Path $ProgramFiles "dig-network"
$VersionDir = Join-Path $InstallBase "v$Version"

Write-Host "Installing to: $VersionDir" -ForegroundColor Green

# Check if already installed
if (Test-Path $VersionDir -and -not $Force) {
    Write-Host "Version $Version is already installed at: $VersionDir" -ForegroundColor Yellow
    Write-Host "Use -Force to reinstall" -ForegroundColor Yellow
    exit 0
}

# Create directories
Write-Host "Creating installation directory..." -ForegroundColor Blue
try {
    New-Item -ItemType Directory -Path $VersionDir -Force | Out-Null
} catch {
    Write-Host "❌ Failed to create directory: $VersionDir" -ForegroundColor Red
    Write-Host "This script requires administrator privileges to install to Program Files." -ForegroundColor Red
    Write-Host "Please run as administrator or install to user directory manually." -ForegroundColor Red
    exit 1
}

# Download the latest MSI
$DownloadUrl = "https://github.com/DIG-Network/digstore/releases/latest/download/digstore-windows-x64.msi"
if ($Version -ne "latest") {
    $DownloadUrl = "https://github.com/DIG-Network/digstore/releases/download/v$Version/digstore-windows-x64.msi"
}

$TempMsi = Join-Path $env:TEMP "digstore-installer.msi"

Write-Host "Downloading from: $DownloadUrl" -ForegroundColor Blue
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempMsi -UserAgent "Digstore-Bootstrap"
    Write-Host "✅ Download completed" -ForegroundColor Green
} catch {
    Write-Host "❌ Download failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

# Install MSI to system location
Write-Host "Installing MSI..." -ForegroundColor Blue
try {
    $InstallArgs = @("/i", $TempMsi, "/quiet", "/norestart")
    $Process = Start-Process "msiexec" -ArgumentList $InstallArgs -Wait -PassThru
    
    if ($Process.ExitCode -ne 0) {
        throw "MSI installation failed with exit code: $($Process.ExitCode)"
    }
    
    Write-Host "✅ MSI installation completed" -ForegroundColor Green
} catch {
    Write-Host "❌ Installation failed: $($_.Exception.Message)" -ForegroundColor Red
    Remove-Item $TempMsi -ErrorAction SilentlyContinue
    exit 1
}

# Wait for installation to complete
Start-Sleep -Seconds 2

# Move installed files to versioned directory
Write-Host "Organizing into versioned directory..." -ForegroundColor Blue
$SourceBinary = Join-Path $InstallBase "digstore.exe"
$TargetBinary = Join-Path $VersionDir "digstore.exe"

if (Test-Path $SourceBinary) {
    # Move binary to versioned directory
    Move-Item $SourceBinary $TargetBinary -Force
    
    # Move any other files
    Get-ChildItem $InstallBase -File | ForEach-Object {
        $TargetPath = Join-Path $VersionDir $_.Name
        Move-Item $_.FullName $TargetPath -Force -ErrorAction SilentlyContinue
    }
    
    Write-Host "✅ Files organized into versioned directory" -ForegroundColor Green
} else {
    Write-Host "❌ Binary not found at expected location: $SourceBinary" -ForegroundColor Red
    Remove-Item $TempMsi -ErrorAction SilentlyContinue
    exit 1
}

# Update PATH
Write-Host "Updating PATH..." -ForegroundColor Blue
try {
    # Get current PATH
    $CurrentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    
    # Remove any existing dig-network entries
    $PathEntries = $CurrentPath -split ";" | Where-Object { $_ -and -not $_.StartsWith($InstallBase) }
    
    # Add new version directory to front
    $NewPath = "$VersionDir;" + ($PathEntries -join ";")
    
    # Update PATH
    [Environment]::SetEnvironmentVariable("PATH", $NewPath, "User")
    $env:PATH = $NewPath
    
    Write-Host "✅ PATH updated to use version $Version" -ForegroundColor Green
} catch {
    Write-Host "⚠️ Could not update PATH automatically: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host "Please manually add to your PATH: $VersionDir" -ForegroundColor Yellow
}

# Save active version
$ActiveFile = Join-Path $InstallBase "active"
Set-Content -Path $ActiveFile -Value $Version -Force

# Clean up
Remove-Item $TempMsi -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "🎉 Digstore $Version installed successfully!" -ForegroundColor Green
Write-Host "Location: $VersionDir" -ForegroundColor Cyan
Write-Host ""
Write-Host "Usage:" -ForegroundColor Yellow
Write-Host "  digstore --version                    # Check installed version"
Write-Host "  digstore init                         # Initialize a repository" 
Write-Host "  digstore version list                 # List installed versions"
Write-Host "  digstore version install-version 0.4.8  # Install specific version"
Write-Host "  digstore version set 0.4.8           # Switch to version"
Write-Host "  digstore update                       # Update to latest"
Write-Host ""
Write-Host "For help: digstore --help" -ForegroundColor Cyan
Write-Host ""
Write-Host "⚠️ Restart your terminal to use the new PATH" -ForegroundColor Yellow
