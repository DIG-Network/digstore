//! Status command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::find_repository_root;
use colored::Colorize;

/// Execute the status command
pub fn execute(
    short: bool,
    porcelain: bool,
    show_chunks: bool,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let store = Store::open(&repo_root)?;
    let status = store.status();

    if porcelain {
        // Machine-readable output
        for file_path in &status.staged_files {
            println!("A {}", file_path.display());
        }
        return Ok(());
    }

    if short {
        // Short format
        if status.staged_files.is_empty() {
            println!("nothing to commit, working tree clean");
        } else {
            for file_path in &status.staged_files {
                println!("A  {}", file_path.display());
            }
        }
        return Ok(());
    }

    // Full status display
    println!("{}", "Repository Status".bright_blue().bold());
    println!("{}", "═".repeat(40));
    println!();

    println!("Store ID: {}", status.store_id.to_hex().cyan());
    
    if let Some(current_root) = status.current_root {
        println!("Current commit: {}", current_root.to_hex().cyan());
    } else {
        println!("Current commit: {} (no commits yet)", "none".dimmed());
    }

    println!();

    if !status.staged_files.is_empty() {
        println!("{}", "Changes to be committed:".green());
        for file_path in &status.staged_files {
            println!("  {} {}", "new file:".green(), file_path.display());
        }
        println!();
        println!("Summary:");
        println!("  Files staged: {}", status.staged_files.len());
        println!("  Total size: {} bytes", status.total_staged_size);
        
        if show_chunks {
            println!("  {} Chunk information not yet implemented", "!".yellow());
        }
    } else {
        println!("{}", "No changes staged for commit".dimmed());
        println!("  {} Use 'digstore add <files>' to stage files", "→".cyan());
    }

    Ok(())
}
