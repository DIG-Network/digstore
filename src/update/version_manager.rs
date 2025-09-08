//! Version management for digstore installations
//!
//! Manages multiple versions of digstore in separate directories to avoid
//! the Windows file locking issue when updating a running binary.

use crate::core::error::{DigstoreError, Result};
use colored::Colorize;
use directories::UserDirs;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Version manager for handling multiple digstore installations
pub struct VersionManager {
    /// Base directory for all digstore versions (~/.digstore-versions)
    versions_dir: PathBuf,
    /// Current active version
    active_version: Option<String>,
}

impl VersionManager {
    /// Create a new version manager
    pub fn new() -> Result<Self> {
        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
        let versions_dir = user_dirs.home_dir().join(".digstore-versions");

        // Create versions directory if it doesn't exist
        if !versions_dir.exists() {
            fs::create_dir_all(&versions_dir)?;
        }

        let active_version = Self::detect_active_version(&versions_dir)?;

        Ok(Self {
            versions_dir,
            active_version,
        })
    }

    /// Install a new version from a binary path
    pub fn install_version(&mut self, version: &str, binary_path: &Path) -> Result<()> {
        println!(
            "  {} Installing digstore version {}...",
            "•".cyan(),
            version.bright_cyan()
        );

        let version_dir = self.get_version_dir(version);
        
        // Create version directory
        fs::create_dir_all(&version_dir)?;
        
        // Copy binary to version directory
        let target_binary = version_dir.join(self.get_binary_name());
        fs::copy(binary_path, &target_binary)?;
        
        // Make executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target_binary)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_binary, perms)?;
        }

        println!(
            "  {} Version {} installed to: {}",
            "✓".green(),
            version.bright_cyan(),
            version_dir.display().to_string().dimmed()
        );

        Ok(())
    }

    /// Set the active version and update PATH/symlinks
    pub fn set_active_version(&mut self, version: &str) -> Result<()> {
        let version_dir = self.get_version_dir(version);
        let binary_path = version_dir.join(self.get_binary_name());

        if !binary_path.exists() {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Version {} is not installed", version),
            });
        }

        // Update the active symlink/shortcut
        self.update_active_link(&binary_path)?;
        
        // Save active version info
        self.save_active_version(version)?;
        self.active_version = Some(version.to_string());

        println!(
            "  {} Active version set to: {}",
            "✓".green(),
            version.bright_cyan()
        );

        Ok(())
    }

    /// Install and activate a new version from cargo build
    pub fn install_from_cargo(&mut self, version: &str, project_dir: &Path) -> Result<()> {
        println!(
            "{}",
            "Installing digstore with versioned directory structure...".bright_blue()
        );

        // First, check if binary is already built
        let binary_path = project_dir
            .join("target")
            .join("release")
            .join(self.get_binary_name());

        if !binary_path.exists() {
            println!("  {} No pre-built binary found, building from source...", "•".cyan());
            
            // Build the project
            let output = Command::new("cargo")
                .args(&["build", "--release"])
                .current_dir(project_dir)
                .output()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to run cargo build: {}", e),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(DigstoreError::ConfigurationError {
                    reason: format!("Cargo build failed: {}", stderr),
                });
            }

            if !binary_path.exists() {
                return Err(DigstoreError::ConfigurationError {
                    reason: "Built binary not found in target/release".to_string(),
                });
            }
        } else {
            println!("  {} Using pre-built binary from target/release", "•".cyan());
        }

        // Install this version
        self.install_version(version, &binary_path)?;
        
        // Set as active version
        self.set_active_version(version)?;

        println!();
        println!("{}", "✓ Installation completed successfully!".green().bold());
        println!(
            "  Version {} is now active",
            version.bright_cyan()
        );

        // Show usage instructions
        self.show_usage_instructions()?;

        Ok(())
    }

    /// Install the current running binary as a managed version
    pub fn install_current_binary(&mut self, version: &str) -> Result<()> {
        println!(
            "{}",
            "Installing current digstore binary with version management...".bright_blue()
        );

        // Get the path to the currently running binary
        let current_exe = std::env::current_exe()
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to get current executable path: {}", e),
            })?;

        println!(
            "  {} Using current binary: {}",
            "•".cyan(),
            current_exe.display().to_string().dimmed()
        );

        // Install this version
        self.install_version(version, &current_exe)?;
        
        // Set as active version
        self.set_active_version(version)?;

        println!();
        println!("{}", "✓ Installation completed successfully!".green().bold());
        println!(
            "  Version {} is now active",
            version.bright_cyan()
        );

        // Show usage instructions
        self.show_usage_instructions()?;

        Ok(())
    }

    /// List all installed versions
    pub fn list_versions(&self) -> Result<Vec<String>> {
        let mut versions = Vec::new();

        if !self.versions_dir.exists() {
            return Ok(versions);
        }

        for entry in fs::read_dir(&self.versions_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(version) = entry.file_name().to_str() {
                    versions.push(version.to_string());
                }
            }
        }

        versions.sort();
        Ok(versions)
    }

    /// Remove an installed version
    pub fn remove_version(&mut self, version: &str) -> Result<()> {
        if Some(version) == self.active_version.as_deref() {
            return Err(DigstoreError::ConfigurationError {
                reason: "Cannot remove the active version".to_string(),
            });
        }

        let version_dir = self.get_version_dir(version);
        if version_dir.exists() {
            fs::remove_dir_all(&version_dir)?;
            println!(
                "  {} Removed version: {}",
                "✓".green(),
                version.bright_cyan()
            );
        } else {
            println!(
                "  {} Version {} is not installed",
                "!".yellow(),
                version.bright_cyan()
            );
        }

        Ok(())
    }

    /// Get the directory for a specific version
    fn get_version_dir(&self, version: &str) -> PathBuf {
        self.versions_dir.join(version)
    }

    /// Get the binary name for the current platform
    fn get_binary_name(&self) -> &'static str {
        if cfg!(windows) {
            "digstore.exe"
        } else {
            "digstore"
        }
    }

    /// Update the active symlink or shortcut
    fn update_active_link(&self, binary_path: &Path) -> Result<()> {
        let link_path = self.get_active_link_path()?;

        // Remove existing link/shortcut
        if link_path.exists() {
            fs::remove_file(&link_path)?;
        }

        // Create new link/shortcut
        #[cfg(windows)]
        {
            // On Windows, create a batch file that calls the active version
            let batch_content = format!(
                "@echo off\n\"{}\" %*\n",
                binary_path.display()
            );
            fs::write(&link_path, batch_content)?;
        }

        #[cfg(unix)]
        {
            // On Unix, create a symlink
            std::os::unix::fs::symlink(binary_path, &link_path)?;
        }

        Ok(())
    }

    /// Get the path for the active version link
    fn get_active_link_path(&self) -> Result<PathBuf> {
        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
        let bin_dir = user_dirs.home_dir().join(".local").join("bin");
        
        // Create bin directory if it doesn't exist
        fs::create_dir_all(&bin_dir)?;

        #[cfg(windows)]
        let link_path = bin_dir.join("digstore.bat");
        #[cfg(not(windows))]
        let link_path = bin_dir.join("digstore");

        Ok(link_path)
    }

    /// Save the active version to a config file
    fn save_active_version(&self, version: &str) -> Result<()> {
        let config_file = self.versions_dir.join("active");
        fs::write(&config_file, version)?;
        Ok(())
    }

    /// Detect the currently active version
    fn detect_active_version(versions_dir: &Path) -> Result<Option<String>> {
        let config_file = versions_dir.join("active");
        if config_file.exists() {
            let version = fs::read_to_string(&config_file)?;
            Ok(Some(version.trim().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Show usage instructions to the user
    fn show_usage_instructions(&self) -> Result<()> {
        let link_path = self.get_active_link_path()?;
        let bin_dir = link_path.parent().unwrap();

        println!();
        println!("{}", "Usage Instructions:".bright_yellow().bold());
        println!("  1. Add the following directory to your PATH:");
        println!("     {}", bin_dir.display().to_string().bright_cyan());
        println!();
        
        #[cfg(windows)]
        {
            println!("  2. For PowerShell, add this to your profile:");
            println!("     $env:PATH += \";{}\"", bin_dir.display());
            println!();
            println!("  3. For Command Prompt, run:");
            println!("     setx PATH \"%PATH%;{}\"", bin_dir.display());
        }
        
        #[cfg(unix)]
        {
            println!("  2. Add this to your ~/.bashrc or ~/.zshrc:");
            println!("     export PATH=\"{}:$PATH\"", bin_dir.display());
        }

        println!();
        println!("  After updating your PATH, restart your terminal and run:");
        println!("     {}", "digstore --version".bright_green());

        Ok(())
    }
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new().expect("Failed to create version manager")
    }
}
