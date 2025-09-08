//! Version management command implementation

use crate::core::error::{DigstoreError, Result};
use crate::update::VersionManager;
use colored::Colorize;
use std::env;

/// Execute the version management command
pub fn execute(subcommand: Option<String>, version: Option<String>) -> Result<()> {
    match subcommand.as_deref() {
        Some("list") => list_versions(),
        Some("install") => install_current_version(),
        Some("install-current") => install_current_binary(),
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
    println!("  {} - Set active version", "digstore version set <version>".green());
    println!("  {} - Remove a version", "digstore version remove <version>".green());
    
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
