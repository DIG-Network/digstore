//! Update command implementation

use crate::update::{check_for_updates, download_and_install_update, VersionManager};
use anyhow::Result;
use colored::Colorize;
use dialoguer::Confirm;

/// Execute the update command
pub fn execute(check_only: bool, force: bool, json: bool) -> Result<()> {
    if json {
        execute_json_mode(check_only, force)
    } else {
        execute_interactive_mode(check_only, force)
    }
}

/// Execute update in interactive mode
fn execute_interactive_mode(check_only: bool, force: bool) -> Result<()> {
    println!("{}", "Checking for updates...".bright_blue());

    let update_info = match check_for_updates() {
        Ok(info) => info,
        Err(e) => {
            println!("{} Failed to check for updates: {}", "✗".red(), e);
            return Ok(());
        },
    };

    if !update_info.update_available {
        println!(
            "{} You're running the latest version: {}",
            "✓".green(),
            update_info.current_version.bright_green()
        );
        return Ok(());
    }

    println!(
        "{} Update available: {} → {}",
        "🎉".bright_yellow(),
        update_info.current_version.dimmed(),
        update_info.latest_version.bright_green().bold()
    );

    if let Some(notes) = &update_info.release_notes {
        if !notes.trim().is_empty() {
            println!();
            println!("{}", "Release Notes:".bright_cyan().bold());
            println!("{}", notes.trim());
        }
    }

    if check_only {
        return Ok(());
    }

    println!();
    let should_update = if force {
        true
    } else if crate::cli::context::CliContext::is_non_interactive() {
        false // Don't auto-update in non-interactive mode unless forced
    } else {
        Confirm::new()
            .with_prompt("Would you like to download and install the update?")
            .default(true)
            .interact()?
    };

    if !should_update {
        println!("Update cancelled.");
        return Ok(());
    }

    if let Some(download_url) = update_info.download_url {
        match download_and_install_update_versioned(&download_url, &update_info.latest_version) {
            Ok(_) => {
                println!();
                println!(
                    "{}",
                    "🎊 Update completed successfully!".bright_green().bold()
                );
                println!(
                    "  {} Version {} is now installed",
                    "→".cyan(),
                    update_info.latest_version.bright_white()
                );
            },
            Err(e) => {
                println!("{} Update failed: {}", "✗".red(), e);
                println!(
                    "  {} You can manually download from: https://github.com/DIG-Network/digstore/releases",
                    "→".cyan()
                );
            },
        }
    } else {
        println!("{} No installer available for your platform", "⚠".yellow());
        println!(
            "  {} Please download manually from: https://github.com/DIG-Network/digstore/releases",
            "→".cyan()
        );
    }

    Ok(())
}

/// Execute update in JSON mode
fn execute_json_mode(check_only: bool, force: bool) -> Result<()> {
    let update_info = check_for_updates()?;

    if check_only || !update_info.update_available {
        // Just output update information
        let output = serde_json::json!({
            "current_version": update_info.current_version,
            "latest_version": update_info.latest_version,
            "update_available": update_info.update_available,
            "download_url": update_info.download_url,
            "release_notes": update_info.release_notes,
            "action": if check_only { "check" } else { "no_update_needed" }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Perform update
    if let Some(download_url) = update_info.download_url {
        match download_and_install_update(&download_url) {
            Ok(_) => {
                let output = serde_json::json!({
                    "action": "update_completed",
                    "old_version": update_info.current_version,
                    "new_version": update_info.latest_version,
                    "status": "success"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            },
            Err(e) => {
                let output = serde_json::json!({
                    "action": "update_failed",
                    "current_version": update_info.current_version,
                    "latest_version": update_info.latest_version,
                    "error": e.to_string(),
                    "manual_download": "https://github.com/DIG-Network/digstore/releases"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            },
        }
    } else {
        let output = serde_json::json!({
            "action": "update_unavailable",
            "current_version": update_info.current_version,
            "latest_version": update_info.latest_version,
            "reason": "No installer available for platform",
            "manual_download": "https://github.com/DIG-Network/digstore/releases"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

/// Download and install update using nvm-style versioned installation
fn download_and_install_update_versioned(download_url: &str, version: &str) -> Result<()> {
    println!("{}", "Downloading update...".bright_blue());
    println!(
        "{}",
        "Installing with nvm-style version management...".bright_green()
    );

    // Use version manager's nvm-style installation
    let mut vm = VersionManager::new()
        .map_err(|e| anyhow::anyhow!("Failed to create version manager: {}", e))?;

    vm.install_version_from_url(version, download_url)
        .map_err(|e| anyhow::anyhow!("Installation failed: {}", e))?;

    Ok(())
}
