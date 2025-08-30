//! Commit command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::find_repository_root;
use colored::Colorize;

/// Execute the commit command
pub fn execute(
    message: String,
    full_layer: bool,
    author: Option<String>,
    date: Option<String>,
    edit: bool,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let mut store = Store::open(&repo_root)?;

    // Check if there are staged files
    let status = store.status();
    if status.staged_files.is_empty() {
        println!("{} No files staged for commit", "!".yellow());
        println!("  {} Use 'digstore add <files>' to stage files first", "→".cyan());
        return Ok(());
    }

    println!("{}", "Creating commit...".bright_blue());
    println!("  {} Staged files: {}", "•".cyan(), status.staged_files.len());
    println!("  {} Total size: {} bytes", "•".cyan(), status.total_staged_size);

    if let Some(ref author_name) = author {
        println!("  {} Author: {}", "•".cyan(), author_name);
    }

    if let Some(ref date_str) = date {
        println!("  {} Date: {}", "•".cyan(), date_str);
    }

    if edit {
        println!("  {} Editor mode not implemented yet", "!".yellow());
    }

    if full_layer {
        println!("  {} Creating full layer (not delta)", "•".cyan());
    }

    // Create the commit
    let commit_id = store.commit(&message)?;

    println!();
    println!("{} Commit created successfully!", "✓".green().bold());
    println!("  {} Commit ID: {}", "→".cyan(), commit_id.to_hex().bright_cyan());
    println!("  {} Message: {}", "→".cyan(), message.bright_white());
    println!("  {} Files: {}", "→".cyan(), status.staged_files.len());
    println!("  {} Size: {} bytes", "→".cyan(), status.total_staged_size);

    // Show layer file location
    let layer_path = store.global_path().join(format!("{}.layer", commit_id.to_hex()));
    println!("  {} Layer file: {}", "→".cyan(), layer_path.display().to_string().dimmed());

    Ok(())
}
