# Test Windows Installer with UI
# This creates an MSI with a basic UI

Write-Host "Creating Windows Installer with UI" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan

# Set environment variables
$env:PROJECT_AUTH = "DIG Network"

# Check if we have the binary
if (-not (Test-Path "target\release\digstore.exe")) {
    Write-Host "Building release binary..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to build. Please run 'cargo build --release' first." -ForegroundColor Red
        exit 1
    }
}

# Check for WiX
$wixPath = "C:\wix"
if (-not (Test-Path "$wixPath\candle.exe")) {
    Write-Host "WiX not found at $wixPath" -ForegroundColor Red
    Write-Host "Please install WiX first or use the full test script" -ForegroundColor Red
    exit 1
}

# Create installer directory
New-Item -ItemType Directory -Force -Path "installer\windows" | Out-Null

# Create WiX source file WITH UI
Write-Host "Creating WiX file with UI..." -ForegroundColor Green
@"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="Digstore Min" Language="1033" Version="0.1.0" 
           Manufacturer="$env:PROJECT_AUTH" UpgradeCode="A3F5C8D9-E2B1-F4A6-C9D8-E7F2A5B8C1D4">
    <Package InstallerVersion="200" Compressed="yes" InstallScope="perMachine" InstallPrivileges="elevated" 
             Description="Content-addressable storage system with Git-like semantics." />
    <MajorUpgrade AllowSameVersionUpgrades="yes" AllowDowngrades="no" 
                  DowngradeErrorMessage="A newer version of [ProductName] is already installed." 
                  Schedule="afterInstallInitialize" />
    <MediaTemplate EmbedCab="yes" />
    
    <!-- Add UI Reference -->
    <UIRef Id="WixUI_FeatureTree" />
    <UIRef Id="WixUI_ErrorProgressText" />
    
    <!-- Skip license dialog since we don't have a license file -->
    <Publish Dialog="WelcomeDlg" Control="Next" Event="NewDialog" Value="CustomizeDlg" Order="2">1</Publish>
    <Publish Dialog="CustomizeDlg" Control="Back" Event="NewDialog" Value="WelcomeDlg" Order="2">1</Publish>
    
    <!-- Custom dialog text -->
    <Property Id="WIXUI_EXITDIALOGOPTIONALTEXT" Value="Digstore has been successfully installed. If you selected 'Add to PATH', please restart your terminal or log out and back in for PATH changes to take effect." />
    
    <Feature Id="ProductFeature" Title="Digstore Min" Level="1" Display="expand" ConfigurableDirectory="INSTALLFOLDER">
      <ComponentGroupRef Id="ProductComponents" />
      <ComponentRef Id="ProgramMenuDir" />
      
      <!-- Separate feature for PATH addition - optional but default enabled -->
      <Feature Id="PathFeature" Title="Add to PATH" Description="Add Digstore to system PATH environment variable" Level="1" Display="expand">
        <ComponentRef Id="Path" />
      </Feature>
    </Feature>
    
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFilesFolder">
        <Directory Id="INSTALLFOLDER" Name="Digstore" />
      </Directory>
      <Directory Id="ProgramMenuFolder" Name="Programs">
        <Directory Id="ProgramMenuDir" Name="Digstore Min" />
      </Directory>
    </Directory>
    
    <ComponentGroup Id="ProductComponents" Directory="INSTALLFOLDER">
      <Component Id="MainExecutable">
        <File Id="digstore.exe" Source="..\..\target\release\digstore.exe" KeyPath="yes">
          <Shortcut Id="startmenuDigstore" Directory="ProgramMenuDir" Name="Digstore Min" 
                    WorkingDirectory="INSTALLFOLDER" Icon="digstore.exe" IconIndex="0" Advertise="yes" />
        </File>
      </Component>
    </ComponentGroup>
    
    <DirectoryRef Id="TARGETDIR">
      <Component Id="Path" Guid="B3F5C8D9-E2B1-F4A6-C9D8-E7F2A5B8C1D5">
        <Environment Id="UpdatePath" Name="PATH" Value="[INSTALLFOLDER]" Permanent="no" Part="last" Action="set" System="yes" />
        <RegistryValue Root="HKCU" Key="Software\[Manufacturer]\[ProductName]" Name="PathComponent" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
    
    <Component Id="ProgramMenuDir" Guid="C3F5C8D9-E2B1-F4A6-C9D8-E7F2A5B8C1D6" Directory="ProgramMenuDir">
      <RemoveFolder Id="ProgramMenuDir" On="uninstall" />
      <RegistryValue Root="HKCU" Key="Software\[Manufacturer]\[ProductName]" Type="string" Value="" KeyPath="yes" />
    </Component>
    
    <Icon Id="digstore.exe" SourceFile="..\..\target\release\digstore.exe" />
    
    <!-- Show success message -->
    <CustomAction Id="ShowSuccessMessage" Script="vbscript">
      <![CDATA[
        MsgBox "Digstore has been installed successfully!" & vbCrLf & vbCrLf & _
               "Installation location: " & Session.Property("INSTALLFOLDER") & vbCrLf & vbCrLf & _
               "Please restart your terminal to use 'digstore' from the command line.", _
               vbInformation, "Installation Complete"
      ]]>
    </CustomAction>
    
    <!-- Uncomment to show message after install -->
    <!-- <InstallExecuteSequence>
      <Custom Action="ShowSuccessMessage" After="InstallFinalize">NOT Installed</Custom>
    </InstallExecuteSequence> -->
  </Product>
</Wix>
"@ | Out-File -FilePath installer\windows\digstore-ui.wxs -Encoding utf8

# Create a simple license file
@"
Digstore Min - Content-addressable storage system

Copyright (c) 2024 DIG Network

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
"@ | Out-File -FilePath installer\windows\license.rtf -Encoding utf8

Write-Host "Compiling installer..." -ForegroundColor Yellow
Push-Location installer\windows
try {
    # Compile with UI extensions
    & "$wixPath\candle.exe" -ext WixUIExtension digstore-ui.wxs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Candle compilation failed" -ForegroundColor Red
        exit 1
    }
    
    Write-Host "Linking installer..." -ForegroundColor Yellow
    & "$wixPath\light.exe" -ext WixUIExtension digstore-ui.wixobj -o digstore-windows-x64-ui.msi
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Light linking failed" -ForegroundColor Red
        exit 1
    }
    
    Write-Host "`nSuccess! Installer with UI created at:" -ForegroundColor Green
    Write-Host (Resolve-Path "digstore-windows-x64-ui.msi").Path -ForegroundColor Cyan
    
    Write-Host "`nTo test the installer:" -ForegroundColor Yellow
    Write-Host "msiexec /i installer\windows\digstore-windows-x64-ui.msi" -ForegroundColor White
    
} finally {
    Pop-Location
}

Write-Host "`nNote: The installer will show:" -ForegroundColor Cyan
Write-Host "- Welcome screen" -ForegroundColor White
Write-Host "- Installation directory selection" -ForegroundColor White
Write-Host "- Progress bar during installation" -ForegroundColor White
Write-Host "- Completion screen" -ForegroundColor White
