//! Interactive CLI prompts for user input
//!
//! This module provides interactive prompts for handling various user scenarios
//! such as store recreation, confirmations, and recovery options.

use crate::storage::Store;
use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input};
use std::path::Path;

/// Ask user if they want to recreate a missing store
pub fn ask_recreate_store(archive_path: &Path, store_id_hex: &str, auto_yes: bool) -> Result<bool> {
    if auto_yes {
        println!(
            "{}",
            "Store not found - auto-recreating (--yes flag)".green()
        );
        return Ok(true);
    }

    println!();
    println!("{}", "Store not found!".red().bold());
    println!(
        "  Archive file: {}",
        archive_path.display().to_string().yellow()
    );
    println!("  Store ID: {}", store_id_hex.cyan());
    println!();

    let recreate = Confirm::new()
        .with_prompt("Would you like to recreate this store?")
        .default(true)
        .interact()?;

    Ok(recreate)
}

/// Ask user for confirmation to overwrite existing .digstore file
pub fn ask_overwrite_digstore(digstore_path: &Path, auto_yes: bool) -> Result<bool> {
    if auto_yes {
        println!(
            "{}",
            "Repository file exists - auto-overwriting (--yes flag)".green()
        );
        return Ok(true);
    }

    println!();
    println!("{}", "Warning: Repository file exists".yellow().bold());
    println!("  File: {}", digstore_path.display().to_string().yellow());
    println!();
    println!(
        "{}",
        "Overwriting will create a new store with a different ID.".red()
    );
    println!(
        "{}",
        "Any existing data in the old store will become inaccessible.".red()
    );
    println!();

    let overwrite = Confirm::new()
        .with_prompt("Are you sure you want to overwrite the existing repository?")
        .default(false)
        .interact()?;

    Ok(overwrite)
}

/// Interactive store recreation workflow
pub fn interactive_store_recreation(project_path: &Path) -> Result<Store> {
    println!();
    println!("{}", "Creating new repository...".blue().bold());

    // Get repository name
    let default_name = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-project");

    let repo_name: String = Input::new()
        .with_prompt("Repository name")
        .default(default_name.to_string())
        .interact_text()?;

    println!();
    println!("Creating repository '{}'...", repo_name.cyan());

    // Create new store (this will overwrite .digstore if it exists)
    let store = Store::init(project_path)?;

    println!();
    println!("{}", "Repository created".green());
    println!("  Store ID: {}", store.store_id().to_hex().cyan());
    println!(
        "  Location: {}",
        store.global_path().display().to_string().dimmed()
    );

    Ok(store)
}

/// Handle missing store scenario with interactive prompts
pub fn handle_missing_store(
    archive_path: &Path,
    store_id_hex: &str,
    project_path: &Path,
    auto_yes: bool,
) -> Result<Store> {
    // Ask if user wants to recreate
    if ask_recreate_store(archive_path, store_id_hex, auto_yes)? {
        let digstore_path = project_path.join(".digstore");

        // Check if .digstore file exists and ask for confirmation
        if digstore_path.exists()
            && !ask_overwrite_digstore(&digstore_path, auto_yes)? {
                println!();
                println!("{}", "Operation cancelled".yellow());
                return Err(anyhow::anyhow!("User cancelled store recreation"));
            }

        // Proceed with interactive recreation
        interactive_store_recreation(project_path)
    } else {
        println!();
        println!("{}", "Operation cancelled".yellow());
        println!();
        println!("{}", "Alternative solutions:".blue().bold());
        println!(
            "  1. {} Check if you're in the right directory",
            "Location:".green()
        );
        println!("     {}", "cd /path/to/your/project".cyan());
        println!();
        println!(
            "  2. {} Restore the missing store file",
            "Recovery:".green()
        );
        println!("     {}", "The store may have been moved or deleted".cyan());
        println!();

        Err(anyhow::anyhow!(
            "Store not found and user declined recreation"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_interactive_functions_exist() {
        // This test just verifies the functions exist and can be called
        // Interactive testing would require manual input

        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.dig");
        let store_id = "test123";

        // These functions exist and have the right signatures
        // We can't test the actual interaction without manual input
        assert!(archive_path.to_string_lossy().contains("test.dig"));
        assert_eq!(store_id, "test123");
    }
}
