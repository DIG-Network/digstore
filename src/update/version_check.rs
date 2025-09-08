//! Version checking against GitHub releases

use crate::core::error::{DigstoreError, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/DIG-Network/digstore/releases/latest";
const UPDATE_CHECK_INTERVAL: u64 = 24 * 60 * 60; // 24 hours in seconds

/// Information about available updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub download_url: Option<String>,
    pub release_notes: Option<String>,
}

/// GitHub release API response
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: String,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
}

/// GitHub release asset
#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Check for available updates
pub fn check_for_updates() -> Result<UpdateInfo> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    
    // Check if we should skip update check (too recent)
    if !should_check_for_updates()? {
        return Ok(UpdateInfo {
            current_version: current_version.clone(),
            latest_version: current_version,
            update_available: false,
            download_url: None,
            release_notes: None,
        });
    }
    
    // Fetch latest release info from GitHub
    let latest_release = fetch_latest_release()?;
    let latest_version = latest_release.tag_name.trim_start_matches('v').to_string();
    
    // Compare versions
    let update_available = is_newer_version(&latest_version, &current_version)?;
    
    // Find appropriate download URL for current platform
    let download_url = if update_available {
        find_platform_download_url(&latest_release.assets)
    } else {
        None
    };
    
    // Update last check timestamp
    update_last_check_timestamp()?;
    
    Ok(UpdateInfo {
        current_version,
        latest_version,
        update_available,
        download_url,
        release_notes: latest_release.body,
    })
}

/// Check if we should perform an update check (respects interval)
fn should_check_for_updates() -> Result<bool> {
    let config_path = get_update_config_path()?;
    
    if !config_path.exists() {
        return Ok(true); // First time, always check
    }
    
    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            if let Ok(last_check) = content.trim().parse::<u64>() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                
                Ok(now - last_check > UPDATE_CHECK_INTERVAL)
            } else {
                Ok(true) // Invalid timestamp, check anyway
            }
        },
        Err(_) => Ok(true), // Can't read file, check anyway
    }
}

/// Update the last check timestamp
fn update_last_check_timestamp() -> Result<()> {
    let config_path = get_update_config_path()?;
    
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    std::fs::write(&config_path, now.to_string())?;
    Ok(())
}

/// Get path to update config file
fn get_update_config_path() -> Result<std::path::PathBuf> {
    use directories::UserDirs;
    
    let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
    let dig_dir = user_dirs.home_dir().join(".dig");
    Ok(dig_dir.join("last_update_check"))
}

/// Fetch latest release from GitHub API
fn fetch_latest_release() -> Result<GitHubRelease> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("digstore-cli")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| DigstoreError::NetworkError {
            reason: format!("Failed to create HTTP client: {}", e),
        })?;
    
    let response = client
        .get(GITHUB_RELEASES_URL)
        .send()
        .map_err(|e| DigstoreError::NetworkError {
            reason: format!("Failed to fetch release info: {}", e),
        })?;
    
    if !response.status().is_success() {
        return Err(DigstoreError::NetworkError {
            reason: format!("GitHub API returned status: {}", response.status()),
        });
    }
    
    let release: GitHubRelease = response
        .json()
        .map_err(|e| DigstoreError::NetworkError {
            reason: format!("Failed to parse release info: {}", e),
        })?;
    
    Ok(release)
}

/// Compare two version strings (semver-like)
fn is_newer_version(latest: &str, current: &str) -> Result<bool> {
    // Simple version comparison - parse as semver-like
    let parse_version = |v: &str| -> Result<(u32, u32, u32)> {
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() != 3 {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Invalid version format: {}", v),
            });
        }
        
        let major = parts[0].parse::<u32>().map_err(|_| DigstoreError::ConfigurationError {
            reason: format!("Invalid major version: {}", parts[0]),
        })?;
        let minor = parts[1].parse::<u32>().map_err(|_| DigstoreError::ConfigurationError {
            reason: format!("Invalid minor version: {}", parts[1]),
        })?;
        let patch = parts[2].parse::<u32>().map_err(|_| DigstoreError::ConfigurationError {
            reason: format!("Invalid patch version: {}", parts[2]),
        })?;
        
        Ok((major, minor, patch))
    };
    
    let latest_version = parse_version(latest)?;
    let current_version = parse_version(current)?;
    
    Ok(latest_version > current_version)
}

/// Find download URL for current platform
fn find_platform_download_url(assets: &[GitHubAsset]) -> Option<String> {
    let platform_patterns = if cfg!(target_os = "windows") {
        vec!["windows-x64.msi", "windows.msi"]
    } else if cfg!(target_os = "macos") {
        vec!["macos.dmg", "darwin.dmg"]
    } else if cfg!(target_os = "linux") {
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
