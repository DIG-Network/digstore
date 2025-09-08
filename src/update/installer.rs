//! Installer and update management

use crate::core::error::{DigstoreError, Result};
use crate::update::version_check::GitHubAsset;
use colored::Colorize;
use std::path::Path;
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
fn download_installer(url: &str) -> Result<std::path::PathBuf> {
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

/// Install macOS DMG
fn install_macos_dmg(dmg_path: &Path) -> Result<()> {
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

    // Copy app to Applications
    let app_name = "DIG Network Digstore.app";
    let source = format!("{}/{}", mount_point, app_name);
    let destination = format!("/Applications/{}", app_name);

    let copy_output = Command::new("cp")
        .args(&["-r", &source, &destination])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to copy app: {}", e),
        })?;

    // Unmount DMG
    let _ = Command::new("hdiutil")
        .args(&["detach", mount_point])
        .output();

    if !copy_output.status.success() {
        return Err(DigstoreError::ConfigurationError {
            reason: "Failed to install app to Applications folder".to_string(),
        });
    }

    Ok(())
}

/// Install Linux DEB package
fn install_linux_deb(deb_path: &Path) -> Result<()> {
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

    Ok(())
}

/// Install Linux RPM package
fn install_linux_rpm(rpm_path: &Path) -> Result<()> {
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

    Ok(())
}

/// Install Linux AppImage
fn install_linux_appimage(appimage_path: &Path) -> Result<()> {
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

    // Copy to /usr/local/bin (requires sudo)
    let copy_output = Command::new("sudo")
        .args(&[
            "cp",
            appimage_path.to_str().unwrap(),
            "/usr/local/bin/digstore",
        ])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to install AppImage: {}", e),
        })?;

    if !copy_output.status.success() {
        let stderr = String::from_utf8_lossy(&copy_output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("AppImage installation failed: {}", stderr),
        });
    }

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
