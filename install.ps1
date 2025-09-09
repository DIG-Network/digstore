# DigStore Bootstrap Installer
# This script downloads and installs the latest version of DigStore with version management

param(
    [string]$Version = "latest",
    [switch]$Force = $false
)

$ErrorActionPreference = "Stop"

Write-Host "🚀 DigStore Bootstrap Installer" -ForegroundColor Cyan
Write-Host "==================================" -ForegroundColor Cyan
Write-Host ""

# Determine installation directory
$ProgramFiles = $env:ProgramFiles
if ($env:PROCESSOR_ARCHITECTURE -eq "AMD64" -and $env:ProgramW6432) {
    $ProgramFiles = ${env:ProgramFiles(x86)}
}

# Try system installation first, fallback to user directory
$SystemInstallDir = Join-Path $ProgramFiles "dig-network"
$UserInstallDir = Join-Path $env:USERPROFILE ".digstore-versions"

$InstallDir = $SystemInstallDir
$IsSystemInstall = $true

# Test write access to system directory
try {
    $TestFile = Join-Path $SystemInstallDir "test_write.tmp"
    New-Item -ItemType Directory -Path $SystemInstallDir -Force -ErrorAction SilentlyContinue | Out-Null
    [System.IO.File]::WriteAllText($TestFile, "test")
    Remove-Item $TestFile -ErrorAction SilentlyContinue
    Write-Host "✅ System installation directory accessible: $SystemInstallDir" -ForegroundColor Green
} catch {
    Write-Host "⚠️  Cannot write to system directory, using user directory: $UserInstallDir" -ForegroundColor Yellow
    $InstallDir = $UserInstallDir
    $IsSystemInstall = $false
}

# Determine download URL based on latest release
$ApiUrl = "https://api.github.com/repos/DIG-Network/digstore/releases/latest"
Write-Host "Fetching release information..." -ForegroundColor Blue

try {
    $Response = Invoke-RestMethod -Uri $ApiUrl -ErrorAction Stop
    
    if ($Version -eq "latest") {
        $Version = $Response.tag_name.TrimStart('v')
        Write-Host "Latest version: $Version" -ForegroundColor Green
    }
    
    # Find Windows binary asset in the release
    $BinaryAsset = $Response.assets | Where-Object { $_.name -match "digstore-windows-x64-v.*\.exe$" }
    if (-not $BinaryAsset) {
        Write-Host "❌ Windows binary not found in latest release" -ForegroundColor Red
        exit 1
    }
    
    $DownloadUrl = $BinaryAsset.browser_download_url
    Write-Host "Download URL: $DownloadUrl" -ForegroundColor Cyan
} catch {
    Write-Host "❌ Failed to fetch release information: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

# Download binary
$TempBinary = Join-Path $env:TEMP "digstore-$Version.exe"
Write-Host "Downloading binary..." -ForegroundColor Yellow

try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempBinary -UserAgent "DigStore-Bootstrap"
    Write-Host "✅ Download completed" -ForegroundColor Green
} catch {
    Write-Host "❌ Download failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

# Create version directory
$VersionDir = Join-Path $InstallDir "v$Version"
Write-Host "Installing to: $VersionDir" -ForegroundColor Blue

try {
    New-Item -ItemType Directory -Path $VersionDir -Force | Out-Null
    Copy-Item $TempBinary -Destination (Join-Path $VersionDir "digstore.exe") -Force
    Write-Host "✅ Binary installed successfully" -ForegroundColor Green
} catch {
    Write-Host "❌ Installation failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

# Update PATH
Write-Host "Updating PATH..." -ForegroundColor Blue
try {
    if ($IsSystemInstall) {
        # System-wide PATH update
        $CurrentPath = [Environment]::GetEnvironmentVariable("PATH", "Machine")
        $NewPath = "$VersionDir;$CurrentPath"
        [Environment]::SetEnvironmentVariable("PATH", $NewPath, "Machine")
        Write-Host "✅ System PATH updated" -ForegroundColor Green
    } else {
        # User PATH update
        $CurrentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
        if ($CurrentPath) {
            $NewPath = "$VersionDir;$CurrentPath"
        } else {
            $NewPath = $VersionDir
        }
        [Environment]::SetEnvironmentVariable("PATH", $NewPath, "User")
        Write-Host "✅ User PATH updated" -ForegroundColor Green
    }
    
    # Update current session PATH
    $env:PATH = "$VersionDir;$env:PATH"
    Write-Host "✅ Current session PATH updated" -ForegroundColor Green
} catch {
    Write-Host "⚠️  PATH update failed: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host "   You may need to add $VersionDir to your PATH manually" -ForegroundColor Yellow
}

# Test installation
Write-Host "Testing installation..." -ForegroundColor Blue
try {
    $TestResult = & (Join-Path $VersionDir "digstore.exe") --version 2>&1
    Write-Host "✅ Installation test successful: $TestResult" -ForegroundColor Green
} catch {
    Write-Host "⚠️  Installation test failed: $($_.Exception.Message)" -ForegroundColor Yellow
}

# Clean up
Remove-Item $TempBinary -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "🎉 DigStore $Version installed successfully!" -ForegroundColor Green
Write-Host "Location: $VersionDir" -ForegroundColor Cyan
Write-Host ""
Write-Host "Usage:" -ForegroundColor Yellow
Write-Host "  digstore --version                    # Check installed version"
Write-Host "  digstore init                         # Initialize a repository" 
Write-Host "  digstore version list                 # List installed versions"
Write-Host "  digstore version set <version>        # Switch to a different version"
Write-Host ""

if (-not $IsSystemInstall) {
    Write-Host "⚠️  Note: Installed to user directory. You may need to restart your terminal" -ForegroundColor Yellow
    Write-Host "   for PATH changes to take effect." -ForegroundColor Yellow
} else {
    Write-Host "ℹ️  Note: You may need to restart your terminal for PATH changes to take effect." -ForegroundColor Cyan
}

Write-Host ""
Write-Host "📚 Documentation: https://github.com/DIG-Network/digstore" -ForegroundColor Cyan