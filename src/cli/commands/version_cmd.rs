//! Version management command implementation

use crate::core::error::{DigstoreError, Result};
use crate::update::VersionManager;
use colored::Colorize;
use std::env;

/// Execute the version management command
pub fn execute(subcommand: Option<String>, version: Option<String>) -> Result<()> {
    match subcommand.as_deref() {
        Some("list") => list_versions(),
        Some("list-system") => list_system_versions(),
        Some("install") => install_current_version(),
        Some("install-current") => install_current_binary(),
        Some("install-msi") => {
            let msi_path = version.ok_or_else(|| DigstoreError::ConfigurationError {
                reason: "MSI path required for 'install-msi' command".to_string(),
            })?;
            install_from_msi(&msi_path)
        }
        Some("set") => {
            let version = version.ok_or_else(|| DigstoreError::ConfigurationError {
                reason: "Version required for 'set' command".to_string(),
            })?;
            set_active_version(&version)
        }
        Some("remove") => {
            let version = version.ok_or_else(|| DigstoreError::ConfigurationError {
                reason: "Version required for 'remove' command".to_string(),
            })?;
            remove_version(&version)
        }
        Some("update-path") => {
            let version = version.ok_or_else(|| DigstoreError::ConfigurationError {
                reason: "Version required for 'update-path' command".to_string(),
            })?;
            update_path_for_version(&version)
        }
        Some("fix-path") => fix_path_ordering(),
        Some("fix-path-auto") => fix_path_ordering_automatically(),
        Some("current") => show_current_version(),
        _ => show_version_info(),
    }
}

/// Show current version information
fn show_version_info() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    
    println!("{}", "Digstore Version Information".bright_blue().bold());
    println!();
    println!("  {} {}", "Current Version:".bright_white(), current_version.bright_cyan());
    
    // Show version manager info if available
    match VersionManager::new() {
        Ok(vm) => {
            match vm.list_versions() {
                Ok(versions) => {
                    if !versions.is_empty() {
                        println!("  {} {}", "Installed Versions:".bright_white(), versions.len());
                        for version in &versions {
                            println!("    • {}", version.dimmed());
                        }
                    }
                }
                Err(_) => {} // Ignore errors for version listing
            }
        }
        Err(_) => {} // Ignore version manager errors
    }
    
    println!();
    println!("{}", "Available Commands:".bright_yellow().bold());
    println!("  {} - Show this version information", "digstore version".green());
    println!("  {} - List all installed versions", "digstore version list".green());
    println!("  {} - Install current version with version manager", "digstore version install".green());
    println!("  {} - Install currently running binary", "digstore version install-current".green());
    println!("  {} - Install from MSI file", "digstore version install-msi <path>".green());
    println!("  {} - List system-installed versions", "digstore version list-system".green());
    println!("  {} - Set active version", "digstore version set <version>".green());
    println!("  {} - Update PATH for version", "digstore version update-path <version>".green());
    println!("  {} - Remove a version", "digstore version remove <version>".green());
    println!("  {} - Fix PATH ordering", "digstore version fix-path".green());
    println!("  {} - Auto-fix PATH ordering", "digstore version fix-path-auto".green());
    
    Ok(())
}

/// List all installed versions
fn list_versions() -> Result<()> {
    let vm = VersionManager::new()?;
    let versions = vm.list_versions()?;
    
    if versions.is_empty() {
        println!("{}", "No versions installed through version manager".yellow());
        println!("Run {} to install the current version", "digstore version install".green());
        return Ok(());
    }
    
    println!("{}", "Installed Versions:".bright_blue().bold());
    println!();
    
    for version in &versions {
        println!("  • {}", version.bright_cyan());
    }
    
    println!();
    println!("Total: {} version(s)", versions.len().to_string().bright_white());
    
    Ok(())
}

/// Install the current version using the version manager
fn install_current_version() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let current_dir = env::current_dir()?;
    
    // Check if we're in the digstore project directory
    if !current_dir.join("Cargo.toml").exists() || 
       !current_dir.join("src").join("main.rs").exists() {
        return Err(DigstoreError::ConfigurationError {
            reason: format!(
                "This command must be run from the digstore project directory.\n\
                 Current directory: {}", 
                current_dir.display()
            ),
        });
    }
    
    let mut vm = VersionManager::new()?;
    vm.install_from_cargo(current_version, &current_dir)?;
    
    Ok(())
}

/// Install the current running binary using the version manager
fn install_current_binary() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    
    let mut vm = VersionManager::new()?;
    vm.install_current_binary(current_version)?;
    
    Ok(())
}

/// List system-installed versions
fn list_system_versions() -> Result<()> {
    let vm = VersionManager::new()?;
    let versions = vm.list_system_versions()?;
    
    if versions.is_empty() {
        println!("{}", "No system versions installed".yellow());
        println!("Run {} to install a version", "digstore version install-msi <path>".green());
        return Ok(());
    }
    
    println!("{}", "System-Installed Versions:".bright_blue().bold());
    println!();
    
    // Show active version
    if let Ok(Some(active)) = vm.get_active_version_from_path() {
        println!("  {} {} (active)", "→".green(), active.bright_cyan());
        
        for version in &versions {
            if version != &active {
                println!("  • {}", version.bright_cyan());
            }
        }
    } else {
        for version in &versions {
            println!("  • {}", version.bright_cyan());
        }
        println!();
        println!("{}", "No version currently active in PATH".yellow());
    }
    
    println!();
    println!("Total: {} version(s)", versions.len().to_string().bright_white());
    
    Ok(())
}

/// Install from MSI file
fn install_from_msi(msi_path: &str) -> Result<()> {
    let msi_file = std::path::Path::new(msi_path);
    
    if !msi_file.exists() {
        return Err(DigstoreError::ConfigurationError {
            reason: format!("MSI file not found: {}", msi_path),
        });
    }
    
    // Extract version from MSI filename
    let version = if let Some(filename) = msi_file.file_name().and_then(|n| n.to_str()) {
        // Try different patterns: "digstore-windows-x64.msi" (no version) or with version
        if filename.contains("digstore-windows-x64.msi") {
            // This is likely the latest version from GitHub releases
            "latest".to_string()
        } else if let Some(start) = filename.find("v") {
            if let Some(end) = filename[start..].find(".msi") {
                let version_part = &filename[start + 1..start + end];
                if version_part.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
                    version_part.to_string()
                } else {
                    "latest".to_string()
                }
            } else {
                "latest".to_string()
            }
        } else {
            // Default to latest for standard GitHub release MSI files
            "latest".to_string()
        }
    } else {
        "latest".to_string()
    };
    
    let mut vm = VersionManager::new()?;
    vm.install_from_msi(&version, msi_file)?;
    
    Ok(())
}

/// Set the active version
fn set_active_version(version: &str) -> Result<()> {
    let mut vm = VersionManager::new()?;
    vm.set_active_version(version)?;
    
    println!();
    println!("{}", "✓ Active version updated successfully!".green().bold());
    println!("Run {} to verify", "digstore --version".dimmed());
    
    Ok(())
}

/// Remove a version
fn remove_version(version: &str) -> Result<()> {
    let mut vm = VersionManager::new()?;
    vm.remove_version(version)?;
    
    println!();
    println!("{}", "✓ Version removed successfully!".green().bold());
    
    Ok(())
}

/// Update PATH to point to a specific version
fn update_path_for_version(version: &str) -> Result<()> {
    let vm = VersionManager::new()?;
    
    // Check if version exists
    let versions = vm.list_versions()?;
    if !versions.contains(&version.to_string()) {
        return Err(DigstoreError::ConfigurationError {
            reason: format!("Version {} is not installed", version),
        });
    }
    
    let version_dir = vm.get_version_dir(version);
    let binary_path = version_dir.join(vm.get_binary_name());
    
    if !binary_path.exists() {
        return Err(DigstoreError::ConfigurationError {
            reason: format!("Binary not found for version {}: {}", version, binary_path.display()),
        });
    }
    
    println!(
        "{}",
        format!("Updating PATH for digstore version {}...", version).bright_blue()
    );
    
    // Get the directory that should be in PATH (where the batch file is)
    let link_path = vm.get_active_link_path()?;
    let bin_dir = link_path.parent().unwrap();
    
    println!("  {} Version directory: {}", "•".cyan(), version_dir.display().to_string().dimmed());
    println!("  {} PATH directory: {}", "•".cyan(), bin_dir.display().to_string().dimmed());
    
    // Update the batch file to point to this version
    let mut vm_mut = vm;
    vm_mut.set_active_version(version)?;
    
    // Show PATH instructions
    println!();
    println!("{}", "PATH Update Instructions:".bright_yellow().bold());
    println!("  Add this directory to your PATH if not already added:");
    println!("     {}", bin_dir.display().to_string().bright_cyan());
    println!();
    
    #[cfg(windows)]
    {
        println!("  For PowerShell, run:");
        println!("     $env:PATH += \";{}\"", bin_dir.display());
        println!();
        println!("  For permanent PATH update, run:");
        println!("     setx PATH \"%PATH%;{}\"", bin_dir.display());
        println!();
        println!("  {} After updating PATH, restart your terminal and run:", "→".cyan());
        println!("     {}", "digstore --version".bright_green());
    }
    
    Ok(())
}

/// Fix PATH ordering to prioritize version-managed digstore
fn fix_path_ordering() -> Result<()> {
    println!(
        "{}",
        "Analyzing PATH for digstore conflicts...".bright_blue()
    );
    
    // Check current PATH
    let current_path = std::env::var("PATH").unwrap_or_default();
    let path_entries: Vec<&str> = current_path.split(';').collect();
    
    let mut digstore_locations = Vec::new();
    
    // Find all directories in PATH that might contain digstore
    for (index, entry) in path_entries.iter().enumerate() {
        let entry_path = std::path::Path::new(entry);
        let digstore_exe = entry_path.join("digstore.exe");
        let digstore_bat = entry_path.join("digstore.bat");
        
        if digstore_exe.exists() || digstore_bat.exists() {
            digstore_locations.push((index, entry, digstore_exe.exists(), digstore_bat.exists()));
        }
    }
    
    if digstore_locations.is_empty() {
        println!("  {} No digstore installations found in PATH", "!".yellow());
        return Ok(());
    }
    
    println!("  {} Found digstore installations:", "•".cyan());
    for (index, path, has_exe, has_bat) in &digstore_locations {
        let file_type = match (has_exe, has_bat) {
            (true, true) => "exe + bat",
            (true, false) => "exe",
            (false, true) => "bat (version-managed)",
            (false, false) => "none",
        };
        println!("    {} Position {}: {} ({})", 
                if *index == 0 { "→".green() } else { "•".dimmed() }, 
                index, 
                path, 
                file_type);
    }
    
    // Check if version-managed directory is first
    let vm = VersionManager::new()?;
    let link_path = vm.get_active_link_path()?;
    let bin_dir = link_path.parent().unwrap();
    
    let version_managed_index = digstore_locations.iter()
        .find(|(_, path, _, has_bat)| *has_bat && std::path::Path::new(path) == bin_dir)
        .map(|(index, _, _, _)| *index);
    
    match version_managed_index {
        Some(0) => {
            println!();
            println!("  {} Version-managed digstore is already first in PATH", "✓".green());
            println!("  {} Current setup is optimal", "✓".green());
        }
        Some(index) => {
            println!();
            println!("  {} Version-managed digstore found at position {}", "!".yellow(), index);
            println!("  {} Earlier installations are taking precedence", "!".yellow());
            println!();
            println!("{}", "Recommended fixes:".bright_yellow().bold());
            println!("  1. {} Remove old installations:", "Option".cyan());
            
            for (i, path, has_exe, _) in &digstore_locations {
                if *i < index && *has_exe {
                    println!("     Remove: {}", path);
                }
            }
            
            println!();
            println!("  2. {} Move version-managed directory to front of PATH:", "Option".cyan());
            println!("     setx PATH \"{};%PATH%\"", bin_dir.display());
        }
        None => {
            println!();
            println!("  {} Version-managed digstore not found in PATH", "!".yellow());
            println!("  {} Add this directory to your PATH:", "→".cyan());
            println!("     {}", bin_dir.display().to_string().bright_cyan());
            println!();
            println!("  {} Run this command:", "→".cyan());
            println!("     setx PATH \"%PATH%;{}\"", bin_dir.display());
        }
    }
    
    Ok(())
}

/// Automatically fix PATH ordering to prioritize version-managed digstore
fn fix_path_ordering_automatically() -> Result<()> {
    println!(
        "{}",
        "Automatically fixing PATH ordering...".bright_blue()
    );
    
    let vm = VersionManager::new()?;
    let link_path = vm.get_active_link_path()?;
    let bin_dir = link_path.parent().unwrap();
    
    // Move version-managed directory to front of PATH
    let current_path = std::env::var("PATH").unwrap_or_default();
    let bin_dir_str = bin_dir.to_string_lossy();
    
    // Remove existing occurrence of this directory from PATH
    let path_entries: Vec<&str> = current_path.split(';').collect();
    let filtered_entries: Vec<&str> = path_entries
        .into_iter()
        .filter(|entry| entry.trim() != bin_dir_str)
        .collect();
    
    // Create new PATH with version-managed directory first
    let new_path = format!("{};{}", bin_dir_str, filtered_entries.join(";"));
    
    println!("  {} Moving {} to front of PATH", "•".cyan(), bin_dir_str);
    
    // Update PATH
    let output = std::process::Command::new("setx")
        .args(&["PATH", &new_path])
        .output()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to update PATH: {}", e),
        })?;
    
    if output.status.success() {
        println!("  {} PATH updated successfully", "✓".green());
        
        // Also update the current environment PATH
        std::env::set_var("PATH", &new_path);
        println!("  {} Current environment PATH refreshed", "✓".green());
        
        println!();
        println!("{}", "✓ PATH ordering fixed!".green().bold());
        println!("  {} Testing new PATH...", "→".cyan());
        
        // Test the new PATH immediately
        match std::process::Command::new("digstore").arg("--version").output() {
            Ok(test_output) if test_output.status.success() => {
                let version_output = String::from_utf8_lossy(&test_output.stdout);
                if let Some(version) = version_output.lines().next().and_then(|line| line.split_whitespace().nth(1)) {
                    println!("  {} Now using version: {}", "✓".green(), version.bright_cyan());
                } else {
                    println!("  {} digstore is now available in PATH", "✓".green());
                }
            }
            _ => {
                println!("  {} PATH updated, but may need terminal restart", "!".yellow());
                println!("  {} Run: {}", "→".cyan(), "digstore --version".bright_green());
            }
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DigstoreError::ConfigurationError {
            reason: format!("Failed to update PATH: {}", stderr),
        });
    }
    
    Ok(())
}

/// Show current active version from version manager
fn show_current_version() -> Result<()> {
    let vm = VersionManager::new()?;
    let versions = vm.list_versions()?;
    
    if versions.is_empty() {
        println!("{}", "No versions managed by version manager".yellow());
        return Ok(());
    }
    
    // The version manager should track the active version
    // For now, just show the running version
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", current_version.bright_cyan());
    
    Ok(())
}
