//! Installer and update management

use crate::core::error::{DigstoreError, Result};
use crate::update::version_check::GitHubAsset;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Types of installers we support
#[derive(Debug, Clone)]
pub enum InstallerType {
    WindowsMsi,
    MacOsDmg,
    LinuxDeb,
    LinuxRpm,
    LinuxAppImage,
}

/// Download and install an update
pub fn download_and_install_update(download_url: &str) -> Result<()> {
    println!("{}", "Downloading update...".bright_blue());

    // Determine installer type from URL
    let installer_type = determine_installer_type(download_url)?;

    // Download the installer
    let temp_file = download_installer(download_url)?;

    println!("{}", "Installing update...".bright_green());

    // Install based on platform
    match installer_type {
        InstallerType::WindowsMsi => install_windows_msi(&temp_file),
        InstallerType::MacOsDmg => install_macos_dmg(&temp_file),
        InstallerType::LinuxDeb => install_linux_deb(&temp_file),
        InstallerType::LinuxRpm => install_linux_rpm(&temp_file),
        InstallerType::LinuxAppImage => install_linux_appimage(&temp_file),
    }?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_file);

    println!("{}", "✓ Update installed successfully!".green().bold());
    println!(
        "{}",
        "Please restart your terminal to use the new version.".yellow()
    );

    Ok(())
}

/// Determine installer type from download URL
fn determine_installer_type(url: &str) -> Result<InstallerType> {
    if url.contains(".msi") {
        Ok(InstallerType::WindowsMsi)
    } else if url.contains(".dmg") {
        Ok(InstallerType::MacOsDmg)
    } else if url.contains(".deb") {
        Ok(InstallerType::LinuxDeb)
    } else if url.contains(".rpm") {
        Ok(InstallerType::LinuxRpm)
    } else if url.contains(".AppImage") {
        Ok(InstallerType::LinuxAppImage)
    } else {
        Err(DigstoreError::ConfigurationError {
            reason: format!("Unsupported installer type for URL: {}", url),
        })
    }
}

/// Download installer to temp file
pub fn download_installer(url: &str) -> Result<std::path::PathBuf> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("digstore-cli")
        .timeout(std::time::Duration::from_secs(300)) // 5 minutes for download
        .build()
        .map_err(|e| DigstoreError::NetworkError {
            reason: format!("Failed to create HTTP client: {}", e),
        })?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| DigstoreError::NetworkError {
            reason: format!("Failed to download installer: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(DigstoreError::NetworkError {
            reason: format!("Download failed with status: {}", response.status()),
        });
    }

    // Create temp file
    let temp_dir = std::env::temp_dir();
    let filename = url.split('/').last().unwrap_or("digstore-installer");
    let temp_file = temp_dir.join(filename);

    // Write downloaded content
    let content = response.bytes().map_err(|e| DigstoreError::NetworkError {
        reason: format!("Failed to read download content: {}", e),
    })?;

    std::fs::write(&temp_file, content)?;

    println!("  {} Downloaded to: {}", "✓".green(), temp_file.display());

    Ok(temp_file)
}

/// Install Windows MSI (headless)
fn install_windows_msi(msi_path: &Path) -> Result<()> {
    let output = Command::new("msiexec")
        .args(&["/i", msi_path.to_str().unwrap(), "/quiet", "/norestart"])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to run msiexec: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("MSI installation failed: {}", stderr),
        });
    }

    Ok(())
}

/// Install macOS DMG with version management
fn install_macos_dmg(dmg_path: &Path) -> Result<()> {
    use crate::update::VersionManager;

    // Mount the DMG
    let mount_output = Command::new("hdiutil")
        .args(&["attach", dmg_path.to_str().unwrap(), "-nobrowse"])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to mount DMG: {}", e),
        })?;

    if !mount_output.status.success() {
        return Err(DigstoreError::ConfigurationError {
            reason: "Failed to mount DMG".to_string(),
        });
    }

    // Extract mount point from output
    let mount_info = String::from_utf8_lossy(&mount_output.stdout);
    let mount_point = mount_info
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().last())
        .ok_or_else(|| DigstoreError::ConfigurationError {
            reason: "Failed to determine mount point".to_string(),
        })?;

    // Extract version from DMG filename
    let version = if let Some(filename) = dmg_path.file_name().and_then(|n| n.to_str()) {
        if filename.contains("digstore-macos.dmg") {
            "latest".to_string()
        } else if let Some(start) = filename.find("v") {
            if let Some(end) = filename[start..].find(".dmg") {
                let version_part = &filename[start + 1..start + end];
                version_part.to_string()
            } else {
                "latest".to_string()
            }
        } else {
            "latest".to_string()
        }
    } else {
        "latest".to_string()
    };

    // Use version manager for organized installation
    let mut vm = VersionManager::new()?;
    let version_dir = vm.get_version_dir(&version);

    // Create versioned directory in system location
    fs::create_dir_all(&version_dir)?;

    // Find the digstore binary in the DMG
    let app_name = "DIG Network Digstore.app";
    let source_app = format!("{}/{}", mount_point, app_name);
    let binary_source = format!("{}/Contents/MacOS/digstore", source_app);
    let binary_target = version_dir.join("digstore");

    // Copy the binary to versioned directory
    let binary_target_str = binary_target.to_string_lossy().to_string();
    let copy_output = Command::new("cp")
        .args(&[&binary_source, &binary_target_str])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to copy binary: {}", e),
        })?;

    // Unmount DMG
    let _ = Command::new("hdiutil")
        .args(&["detach", mount_point])
        .output();

    if !copy_output.status.success() {
        return Err(DigstoreError::ConfigurationError {
            reason: "Failed to copy digstore binary from DMG".to_string(),
        });
    }

    // Make binary executable
    let _ = Command::new("chmod")
        .args(&["+x", &binary_target.to_string_lossy()])
        .output();

    println!(
        "  {} macOS version {} installed to: {}",
        "✓".green(),
        version.bright_cyan(),
        version_dir.display()
    );

    // Update PATH and set as active
    vm.update_path_to_version(&version)?;
    vm.set_active_version(&version)?;

    Ok(())
}

/// Install Linux DEB package with version management
fn install_linux_deb(deb_path: &Path) -> Result<()> {
    use crate::update::VersionManager;

    // Extract version from DEB filename
    let version = if let Some(filename) = deb_path.file_name().and_then(|n| n.to_str()) {
        if filename.contains("digstore_") {
            // Extract version from filename like "digstore_0.4.7_amd64.deb"
            if let Some(start) = filename.find("_") {
                if let Some(end) = filename[start + 1..].find("_") {
                    let version_part = &filename[start + 1..start + 1 + end];
                    version_part.to_string()
                } else {
                    "latest".to_string()
                }
            } else {
                "latest".to_string()
            }
        } else {
            "latest".to_string()
        }
    } else {
        "latest".to_string()
    };

    // Install DEB package to system location first
    let output = Command::new("sudo")
        .args(&["dpkg", "-i", deb_path.to_str().unwrap()])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to run dpkg: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("DEB installation failed: {}", stderr),
        });
    }

    // Use version manager to organize the installation
    let mut vm = VersionManager::new()?;
    let version_dir = vm.get_version_dir(&version);
    fs::create_dir_all(&version_dir)?;

    // Check common DEB installation locations and move to versioned directory
    let deb_locations = [
        PathBuf::from("/usr/local/bin/digstore"),
        PathBuf::from("/usr/bin/digstore"),
        PathBuf::from("/opt/digstore/bin/digstore"),
    ];

    let mut found = false;
    for deb_location in &deb_locations {
        if deb_location.exists() {
            let target_binary = version_dir.join("digstore");

            // Copy to versioned directory
            let copy_output = Command::new("sudo")
                .args(&[
                    "cp",
                    &deb_location.to_string_lossy(),
                    &target_binary.to_string_lossy().to_string(),
                ])
                .output()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to copy binary: {}", e),
                })?;

            if copy_output.status.success() {
                // Make sure it's executable
                let _ = Command::new("sudo")
                    .args(&["chmod", "+x", &target_binary.to_string_lossy().to_string()])
                    .output();

                println!(
                    "  {} Linux version {} installed to: {}",
                    "✓".green(),
                    version.bright_cyan(),
                    version_dir.display()
                );
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(DigstoreError::ConfigurationError {
            reason: "Could not find installed digstore binary after DEB installation".to_string(),
        });
    }

    // Update PATH and set as active
    vm.update_path_to_version(&version)?;
    vm.set_active_version(&version)?;

    Ok(())
}

/// Install Linux RPM package with version management
fn install_linux_rpm(rpm_path: &Path) -> Result<()> {
    use crate::update::VersionManager;

    // Extract version from RPM filename
    let version = if let Some(filename) = rpm_path.file_name().and_then(|n| n.to_str()) {
        if filename.contains("digstore-") {
            // Extract version from filename like "digstore-0.4.7-1.x86_64.rpm"
            if let Some(start) = filename.find("-") {
                if let Some(end) = filename[start + 1..].find("-") {
                    let version_part = &filename[start + 1..start + 1 + end];
                    version_part.to_string()
                } else {
                    "latest".to_string()
                }
            } else {
                "latest".to_string()
            }
        } else {
            "latest".to_string()
        }
    } else {
        "latest".to_string()
    };

    // Install RPM package to system location first
    let output = Command::new("sudo")
        .args(&["rpm", "-U", rpm_path.to_str().unwrap()])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to run rpm: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("RPM installation failed: {}", stderr),
        });
    }

    // Use version manager to organize the installation
    let mut vm = VersionManager::new()?;
    let version_dir = vm.get_version_dir(&version);
    fs::create_dir_all(&version_dir)?;

    // Check common RPM installation locations and move to versioned directory
    let rpm_locations = [
        PathBuf::from("/usr/local/bin/digstore"),
        PathBuf::from("/usr/bin/digstore"),
        PathBuf::from("/opt/digstore/bin/digstore"),
    ];

    let mut found = false;
    for rpm_location in &rpm_locations {
        if rpm_location.exists() {
            let target_binary = version_dir.join("digstore");

            // Copy to versioned directory
            let copy_output = Command::new("sudo")
                .args(&[
                    "cp",
                    &rpm_location.to_string_lossy(),
                    &target_binary.to_string_lossy().to_string(),
                ])
                .output()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to copy binary: {}", e),
                })?;

            if copy_output.status.success() {
                // Make sure it's executable
                let _ = Command::new("sudo")
                    .args(&["chmod", "+x", &target_binary.to_string_lossy().to_string()])
                    .output();

                println!(
                    "  {} Linux version {} installed to: {}",
                    "✓".green(),
                    version.bright_cyan(),
                    version_dir.display()
                );
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(DigstoreError::ConfigurationError {
            reason: "Could not find installed digstore binary after RPM installation".to_string(),
        });
    }

    // Update PATH and set as active
    vm.update_path_to_version(&version)?;
    vm.set_active_version(&version)?;

    Ok(())
}

/// Install Linux AppImage with version management
fn install_linux_appimage(appimage_path: &Path) -> Result<()> {
    use crate::update::VersionManager;

    // Extract version from AppImage filename
    let version = if let Some(filename) = appimage_path.file_name().and_then(|n| n.to_str()) {
        if filename.contains("digstore-linux-x86_64.AppImage") {
            "latest".to_string()
        } else if let Some(start) = filename.find("v") {
            if let Some(end) = filename[start..].find(".AppImage") {
                let version_part = &filename[start + 1..start + end];
                version_part.to_string()
            } else {
                "latest".to_string()
            }
        } else {
            "latest".to_string()
        }
    } else {
        "latest".to_string()
    };

    // Make executable
    let chmod_output = Command::new("chmod")
        .args(&["+x", appimage_path.to_str().unwrap()])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to make AppImage executable: {}", e),
        })?;

    if !chmod_output.status.success() {
        return Err(DigstoreError::ConfigurationError {
            reason: "Failed to make AppImage executable".to_string(),
        });
    }

    // Use version manager for organized installation
    let mut vm = VersionManager::new()?;
    let version_dir = vm.get_version_dir(&version);
    fs::create_dir_all(&version_dir)?;

    let target_binary = version_dir.join("digstore");

    // Copy AppImage to versioned directory
    let copy_output = Command::new("sudo")
        .args(&[
            "cp",
            appimage_path.to_str().unwrap(),
            &target_binary.to_string_lossy().to_string(),
        ])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to copy AppImage: {}", e),
        })?;

    if !copy_output.status.success() {
        let stderr = String::from_utf8_lossy(&copy_output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("AppImage installation failed: {}", stderr),
        });
    }

    // Ensure it's executable
    let _ = Command::new("sudo")
        .args(&["chmod", "+x", &target_binary.to_string_lossy().to_string()])
        .output();

    println!(
        "  {} Linux AppImage version {} installed to: {}",
        "✓".green(),
        version.bright_cyan(),
        version_dir.display()
    );

    // Update PATH and set as active
    vm.update_path_to_version(&version)?;
    vm.set_active_version(&version)?;

    Ok(())
}

/// Find appropriate download URL for current platform
fn find_platform_download_url(assets: &[GitHubAsset]) -> Option<String> {
    let platform_patterns = if cfg!(target_os = "windows") {
        vec!["windows-x64.msi", "windows.msi"]
    } else if cfg!(target_os = "macos") {
        vec!["macos.dmg", "darwin.dmg"]
    } else if cfg!(target_os = "linux") {
        // Prefer AppImage for easier installation, fallback to DEB
        vec!["linux-x86_64.AppImage", "linux.AppImage", "amd64.deb"]
    } else {
        vec![]
    };

    for pattern in platform_patterns {
        for asset in assets {
            if asset.name.contains(pattern) {
                return Some(asset.browser_download_url.clone());
            }
        }
    }

    None
}
