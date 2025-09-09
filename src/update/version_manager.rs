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
        // Try system directory first, fall back to user directory if no admin privileges
        let program_files = std::env::var("ProgramFiles(x86)")
            .or_else(|_| std::env::var("ProgramFiles"))
            .unwrap_or_else(|_| "C:\\Program Files".to_string());
        
        let system_versions_dir = PathBuf::from(program_files).join("dig-network");
        
        // Test if we can write to system directory
        let versions_dir = if Self::can_write_to_directory(&system_versions_dir) {
            // Use system directory (preferred)
            system_versions_dir
        } else {
            // Fall back to user directory
            let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
            user_dirs.home_dir().join(".digstore-versions")
        };

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

    /// Test if we can write to a directory
    fn can_write_to_directory(dir: &Path) -> bool {
        // Try to create the directory and write a test file
        if fs::create_dir_all(dir).is_ok() {
            let test_file = dir.join("access_test.tmp");
            if fs::write(&test_file, "test").is_ok() {
                let _ = fs::remove_file(&test_file);
                return true;
            }
        }
        false
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

    /// Set the active version and update PATH
    pub fn set_active_version(&mut self, version: &str) -> Result<()> {
        let version_dir = self.get_version_dir(version);
        let binary_path = version_dir.join(self.get_binary_name());

        if !binary_path.exists() {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Version {} is not installed", version),
            });
        }

        // Update PATH to point to this version
        self.update_path_to_version(version)?;
        
        // Save active version info
        self.save_active_version(version)?;
        self.active_version = Some(version.to_string());

        println!(
            "  {} Active version set to: {}",
            "✓".green(),
            version.bright_cyan()
        );

        // Test if the version change is immediately effective
        println!("  {} Testing immediate availability...", "→".cyan());
        match std::process::Command::new("digstore").arg("--version").output() {
            Ok(test_output) if test_output.status.success() => {
                let version_output = String::from_utf8_lossy(&test_output.stdout);
                if let Some(detected_version) = version_output.lines().next().and_then(|line| line.split_whitespace().nth(1)) {
                    if detected_version == version {
                        println!("  {} Version {} is now immediately available!", "✓".green(), detected_version.bright_cyan());
                    } else {
                        println!("  {} Currently using version {} (restart terminal for {})", "!".yellow(), detected_version, version);
                    }
                }
            }
            _ => {
                println!("  {} Version available after terminal restart", "→".cyan());
            }
        }

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
    pub fn get_version_dir(&self, version: &str) -> PathBuf {
        self.versions_dir.join(format!("v{}", version))
    }

    /// Get the binary name for the current platform
    pub fn get_binary_name(&self) -> &'static str {
        if cfg!(windows) {
            "digstore.exe"
        } else {
            "digstore"
        }
    }

    /// Update the active symlink or shortcut (no longer needed with direct PATH approach)
    fn update_active_link(&self, binary_path: &Path) -> Result<()> {
        // With the new system versioned approach, we update PATH directly instead of using batch files
        // This method is kept for compatibility but does nothing
        Ok(())
    }

    /// Get the path for the active version link
    pub fn get_active_link_path(&self) -> Result<PathBuf> {
        #[cfg(windows)]
        {
            // Try system-wide first, fall back to user-level
            let program_files = std::env::var("ProgramFiles(x86)")
                .or_else(|_| std::env::var("ProgramFiles"))
                .unwrap_or_else(|_| "C:\\Program Files".to_string());
            
            let system_bin_dir = PathBuf::from(program_files).join("dig-network");
            
            // Test if we can write to system directory
            if fs::create_dir_all(&system_bin_dir).is_ok() {
                let test_file = system_bin_dir.join("access_test.tmp");
                if fs::write(&test_file, "test").is_ok() {
                    let _ = fs::remove_file(&test_file);
                    return Ok(system_bin_dir.join("digstore.bat"));
                }
            }
            
            // Fall back to user directory
            let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
            let user_bin_dir = user_dirs.home_dir().join(".local").join("bin");
            fs::create_dir_all(&user_bin_dir)?;
            Ok(user_bin_dir.join("digstore.bat"))
        }
        
        #[cfg(not(windows))]
        {
            let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
            let bin_dir = user_dirs.home_dir().join(".local").join("bin");
            
            // Create bin directory if it doesn't exist
            fs::create_dir_all(&bin_dir)?;
            
            Ok(bin_dir.join("digstore"))
        }
    }

    /// Get the system-wide installation directory for a version (now same as get_version_dir)
    pub fn get_system_install_dir(&self, version: &str) -> PathBuf {
        self.get_version_dir(version)
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

    /// Install a version from an MSI file to the user versioned directory
    pub fn install_from_msi(&mut self, version: &str, msi_path: &Path) -> Result<()> {
        println!(
            "{}",
            format!("Installing digstore {} from MSI to user directory...", version).bright_blue()
        );

        // Always use user-level versioned installation to avoid conflicts
        self.install_from_msi_user_level(version, msi_path)
    }

    /// Install from MSI to system versioned directory
    fn install_from_msi_user_level(&mut self, version: &str, msi_path: &Path) -> Result<()> {
        println!("  {} Installing MSI to system versioned directory", "•".cyan());
        
        let version_dir = self.get_version_dir(version);
        fs::create_dir_all(&version_dir)?;
        
        // First, install MSI to the default location (where it wants to go)
        println!("  {} Installing MSI to system location...", "•".cyan());
        
        let install_output = Command::new("msiexec")
            .args(&[
                "/i", msi_path.to_str().unwrap(),  // Install the MSI
                "/quiet", "/norestart",            // Silent installation
            ])
            .output()
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to run MSI installation: {}", e),
            })?;

        if !install_output.status.success() {
            let stderr = String::from_utf8_lossy(&install_output.stderr);
            let stdout = String::from_utf8_lossy(&install_output.stdout);
            
            return Err(DigstoreError::ConfigurationError {
                reason: format!("MSI installation failed. Stderr: {}, Stdout: {}", stderr, stdout),
            });
        }

        // Now move the installed binary to the versioned directory
        println!("  {} Moving installation to versioned directory...", "•".cyan());
        
        let base_install_dir = self.versions_dir.clone(); // C:\Program Files (x86)\dig-network\
        let source_binary = base_install_dir.join(self.get_binary_name());
        let target_binary = version_dir.join(self.get_binary_name());
        
        // Check if binary was installed to base directory
        if source_binary.exists() {
            // Move the binary to the versioned directory
            fs::copy(&source_binary, &target_binary)?;
            
            // Remove from base directory
            let _ = fs::remove_file(&source_binary);
            
            // Also move any other files (like DIG.ico)
            if let Ok(entries) = fs::read_dir(&base_install_dir) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_file() && entry_path != source_binary {
                        let filename = entry_path.file_name().unwrap();
                        let target_path = version_dir.join(filename);
                        let _ = fs::copy(&entry_path, &target_path);
                        let _ = fs::remove_file(&entry_path);
                    }
                }
            }
            
            println!(
                "  {} Version {} installed to: {}",
                "✓".green(),
                version.bright_cyan(),
                version_dir.display().to_string().dimmed()
            );
        } else {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("MSI installation succeeded but binary not found at: {}", source_binary.display()),
            });
        }

        // Update PATH to point to the versioned directory
        self.update_path_to_version(version)?;

        // Set as active version
        self.set_active_version(version)?;

        Ok(())
    }

    /// Fallback MSI extraction method when direct installation fails
    fn fallback_msi_extraction(&mut self, version: &str, msi_path: &Path) -> Result<()> {
        println!("  {} Using MSI extraction method...", "•".cyan());
        
        let user_install_dir = self.get_version_dir(version);
        let temp_extract_dir = std::env::temp_dir().join(format!("digstore_extract_{}", version));
        fs::create_dir_all(&temp_extract_dir)?;
        
        // Extract MSI contents using administrative install
        let extract_output = Command::new("msiexec")
            .args(&[
                "/a", msi_path.to_str().unwrap(),  // Administrative install (extract only)
                "/quiet",
                &format!("TARGETDIR={}", temp_extract_dir.display()),
            ])
            .output()
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to extract MSI: {}", e),
            })?;

        let binary_path = user_install_dir.join(self.get_binary_name());
        let mut found = false;

        if extract_output.status.success() {
            found = self.find_and_copy_binary(&temp_extract_dir, &binary_path)?;
        }

        // Cleanup temp directory
        let _ = fs::remove_dir_all(&temp_extract_dir);

        if !found {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Could not extract digstore.exe from MSI"),
            });
        }

        println!(
            "  {} Version {} extracted to: {}",
            "✓".green(),
            version.bright_cyan(),
            user_install_dir.display().to_string().dimmed()
        );

        Ok(())
    }

    /// Clean up system installations and PATH entries
    fn cleanup_system_installations(&self) -> Result<()> {
        println!("  {} Cleaning up system installations...", "•".cyan());
        
        // Remove system binaries
        let system_locations = [
            PathBuf::from("C:\\Program Files (x86)\\dig-network\\digstore.exe"),
            PathBuf::from("C:\\Program Files\\dig-network\\digstore.exe"),
            PathBuf::from("C:\\Program Files (x86)\\DIG Network\\Digstore\\digstore.exe"),
            PathBuf::from("C:\\Program Files\\DIG Network\\Digstore\\digstore.exe"),
        ];

        let mut removed_any = false;
        for system_path in &system_locations {
            if system_path.exists() {
                println!("  {} Removing system installation: {}", "•".cyan(), system_path.display());
                if fs::remove_file(system_path).is_ok() {
                    removed_any = true;
                    
                    // Try to remove parent directory if empty
                    if let Some(parent) = system_path.parent() {
                        let _ = fs::remove_dir(parent);
                    }
                }
            }
        }

        if removed_any {
            println!("  {} Removed conflicting system installations", "✓".green());
        }

        // Clean up PATH entries
        self.cleanup_system_path_entries()?;

        Ok(())
    }

    /// Recursively find and copy the digstore binary from extracted MSI
    fn find_and_copy_binary(&self, search_dir: &Path, target_path: &Path) -> Result<bool> {
        if let Ok(entries) = fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                
                if entry_path.is_file() && entry.file_name() == self.get_binary_name() {
                    fs::copy(&entry_path, target_path)?;
                    return Ok(true);
                }
                
                if entry_path.is_dir() {
                    if self.find_and_copy_binary(&entry_path, target_path)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// Refresh the current environment PATH to include version-managed directory
    pub fn refresh_current_environment(&self) -> Result<()> {
        let link_path = self.get_active_link_path()?;
        let bin_dir = link_path.parent().unwrap();
        let bin_dir_str = bin_dir.to_string_lossy();
        
        // Get current PATH
        let current_path = std::env::var("PATH").unwrap_or_default();
        
        // Check if our directory is already first
        if current_path.starts_with(&format!("{};", bin_dir_str)) {
            return Ok(()); // Already first
        }
        
        // Remove existing occurrence and add to front
        let path_entries: Vec<&str> = current_path.split(';').collect();
        let filtered_entries: Vec<&str> = path_entries
            .into_iter()
            .filter(|entry| entry.trim() != bin_dir_str)
            .collect();
        
        let new_path = format!("{};{}", bin_dir_str, filtered_entries.join(";"));
        std::env::set_var("PATH", &new_path);
        
        Ok(())
    }

    /// Check if we have administrator privileges
    fn has_admin_privileges(&self) -> bool {
        #[cfg(windows)]
        {
            // Try to create a file in Program Files to test admin privileges
            let program_files = std::env::var("ProgramFiles(x86)")
                .or_else(|_| std::env::var("ProgramFiles"))
                .unwrap_or_else(|_| "C:\\Program Files".to_string());
            
            let test_path = PathBuf::from(program_files).join("dig-network").join("admin_test.tmp");
            
            if let Some(parent) = test_path.parent() {
                if fs::create_dir_all(parent).is_ok() {
                    if fs::write(&test_path, "test").is_ok() {
                        let _ = fs::remove_file(&test_path);
                        return true;
                    }
                }
            }
            false
        }
        
        #[cfg(not(windows))]
        {
            // On Unix, check if we can write to /usr/local
            fs::write("/usr/local/digstore_admin_test.tmp", "test").is_ok()
        }
    }

    /// Update system PATH to point to a specific version
    pub fn update_system_path(&self, version: &str) -> Result<()> {
        let install_dir = self.get_system_install_dir(version);
        let binary_path = install_dir.join(self.get_binary_name());

        // Verify binary exists
        if !binary_path.exists() {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Version {} binary not found at: {}", version, binary_path.display()),
            });
        }

        #[cfg(windows)]
        {
            // Update system PATH on Windows
            let output = Command::new("setx")
                .args(&[
                    "/M", // Machine-wide
                    "PATH", 
                    &format!("{};%PATH%", install_dir.display())
                ])
                .output()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to update system PATH: {}", e),
                })?;

            if !output.status.success() {
                // Try user-level PATH update as fallback
                let user_output = Command::new("setx")
                    .args(&[
                        "PATH", 
                        &format!("{};%PATH%", install_dir.display())
                    ])
                    .output()
                    .map_err(|e| DigstoreError::ConfigurationError {
                        reason: format!("Failed to update user PATH: {}", e),
                    })?;

                if !user_output.status.success() {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "Failed to update both system and user PATH".to_string(),
                    });
                }
                
                println!("  {} Updated user PATH (system PATH update requires admin)", "✓".yellow());
            } else {
                println!("  {} Updated system PATH", "✓".green());
            }
        }

        #[cfg(not(windows))]
        {
            // On Unix systems, we can't automatically update PATH
            println!("  {} Add this to your shell profile:", "→".cyan());
            println!("     export PATH=\"{}:$PATH\"", install_dir.display());
        }

        Ok(())
    }

    /// Update PATH to point to a specific version directory
    fn update_path_to_version(&self, version: &str) -> Result<()> {
        let version_dir = self.get_version_dir(version);
        
        println!("  {} Updating PATH to: {}", "•".cyan(), version_dir.display());
        
        // Get current PATH
        let current_path = std::env::var("PATH").unwrap_or_default();
        let path_entries: Vec<&str> = current_path.split(';').collect();
        
        // Remove any existing dig-network entries
        let base_dir_str = self.versions_dir.to_string_lossy();
        let version_dir_str = version_dir.to_string_lossy();
        
        let filtered_entries: Vec<&str> = path_entries
            .into_iter()
            .filter(|entry| {
                let entry_trimmed = entry.trim();
                // Remove old dig-network entries (including versioned ones)
                !entry_trimmed.starts_with(&base_dir_str.to_string())
            })
            .collect();
        
        // Add the new version directory to the front of PATH
        let new_path = format!("{};{}", version_dir_str, filtered_entries.join(";"));
        
        // Update PATH
        let output = Command::new("setx")
            .args(&["PATH", &new_path])
            .output()
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to update PATH: {}", e),
            })?;
        
        if output.status.success() {
            println!("  {} Updated PATH to use version {}", "✓".green(), version.bright_cyan());
            
            // Also update current environment
            std::env::set_var("PATH", &new_path);
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("  {} Could not update PATH automatically: {}", "!".yellow(), stderr);
            println!("  {} Manually add to PATH: {}", "→".cyan(), version_dir.display());
        }
        
        Ok(())
    }

    /// Clean up system PATH entries that point to old installation locations
    fn cleanup_system_path_entries(&self) -> Result<()> {
        println!("  {} Cleaning up old PATH entries...", "•".cyan());
        
        let current_path = std::env::var("PATH").unwrap_or_default();
        let path_entries: Vec<&str> = current_path.split(';').collect();
        
        // System locations to remove from PATH
        let system_locations_to_remove = [
            "C:\\Program Files (x86)\\dig-network",
            "C:\\Program Files\\dig-network",
            "C:\\Program Files (x86)\\DIG Network\\Digstore",
            "C:\\Program Files\\DIG Network\\Digstore",
        ];
        
        let original_count = path_entries.len();
        
        // Filter out old system locations
        let cleaned_entries: Vec<&str> = path_entries
            .into_iter()
            .filter(|entry| {
                let entry_trimmed = entry.trim();
                !system_locations_to_remove.iter().any(|&sys_loc| entry_trimmed == sys_loc)
            })
            .collect();
        
        // Check if any entries were removed
        if cleaned_entries.len() < original_count {
            let new_path = cleaned_entries.join(";");
            
            // Update PATH to remove old system entries
            let output = Command::new("setx")
                .args(&["PATH", &new_path])
                .output()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to update PATH: {}", e),
                })?;
            
            if output.status.success() {
                println!("  {} Removed old system PATH entries", "✓".green());
                
                // Also update current environment
                std::env::set_var("PATH", &new_path);
            } else {
                println!("  {} Could not update PATH automatically", "!".yellow());
                println!("  {} You may need to manually remove old PATH entries:", "→".cyan());
                for location in &system_locations_to_remove {
                    println!("    Remove: {}", location);
                }
            }
        } else {
            println!("  {} No old PATH entries found to clean up", "✓".green());
        }
        
        Ok(())
    }

    /// List all system-installed versions
    pub fn list_system_versions(&self) -> Result<Vec<String>> {
        let mut versions = Vec::new();

        #[cfg(windows)]
        let base_dir = {
            let program_files = std::env::var("ProgramFiles(x86)")
                .or_else(|_| std::env::var("ProgramFiles"))
                .unwrap_or_else(|_| "C:\\Program Files".to_string());
            PathBuf::from(program_files).join("dig-network")
        };

        #[cfg(not(windows))]
        let base_dir = PathBuf::from("/usr/local/lib/digstore");

        if !base_dir.exists() {
            return Ok(versions);
        }

        for entry in fs::read_dir(&base_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(dir_name) = entry.file_name().to_str() {
                    if dir_name.starts_with('v') && dir_name.len() > 1 {
                        let version = &dir_name[1..]; // Remove 'v' prefix
                        
                        // Verify binary exists
                        let binary_path = entry.path().join(self.get_binary_name());
                        if binary_path.exists() {
                            versions.push(version.to_string());
                        }
                    }
                }
            }
        }

        versions.sort();
        Ok(versions)
    }

    /// Check which version is currently active in PATH
    pub fn get_active_version_from_path(&self) -> Result<Option<String>> {
        // Try to run digstore --version to see what's in PATH
        let output = Command::new("digstore")
            .arg("--version")
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let version_output = String::from_utf8_lossy(&output.stdout);
                // Parse "digstore X.Y.Z" format
                if let Some(version) = version_output
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                {
                    Ok(Some(version.to_string()))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None), // digstore not in PATH or error
        }
    }
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new().expect("Failed to create version manager")
    }
}
