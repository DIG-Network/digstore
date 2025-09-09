//! Commit command implementation

use crate::cli::commands::find_repository_root;
use crate::storage::store::Store;
use anyhow::Result;
use colored::Colorize;

/// Execute the commit command
pub fn execute(
    message: String,
    author: Option<String>,
    date: Option<String>,
    edit: bool,
    json: bool,
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
        println!(
            "  {} Use 'digstore add <files>' to stage files first",
            "→".cyan()
        );
        return Ok(());
    }

    println!("{}", "Creating commit...".bright_blue());
    println!(
        "  {} Staged files: {}",
        "•".cyan(),
        status.staged_files.len()
    );
    println!(
        "  {} Total size: {} bytes",
        "•".cyan(),
        status.total_staged_size
    );

    if let Some(ref author_name) = author {
        println!("  {} Author: {}", "•".cyan(), author_name);
    }

    if let Some(ref date_str) = date {
        println!("  {} Date: {}", "•".cyan(), date_str);
    }

    let final_message = if edit {
        // Open editor for commit message
        edit_commit_message(&message)?
    } else {
        message
    };

    // Create the commit
    let commit_id = store.commit(&final_message)?;

    println!();
    if json {
        // JSON output
        let output = serde_json::json!({
            "commit_id": commit_id.to_hex(),
            "message": final_message,
            "files": status.staged_files.len(),
            "size": status.total_staged_size,
            "archive_file": store.archive.path().display().to_string(),
            "layer_id": commit_id.to_hex(),
            "author": author,
            "date": date
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{} Commit created", "✓".green());
        println!(
            "  {} Commit ID: {}",
            "→".cyan(),
            commit_id.to_hex().bright_cyan()
        );
        println!("  {} Message: {}", "→".cyan(), final_message.bright_white());
        println!("  {} Files: {}", "→".cyan(), status.staged_files.len());
        println!("  {} Size: {} bytes", "→".cyan(), status.total_staged_size);

        // Show archive file location (layers are stored inside the archive)
        println!(
            "  {} Archive file: {}",
            "→".cyan(),
            store.archive.path().display().to_string().dimmed()
        );
        println!("  {} Layer ID: {}", "→".cyan(), commit_id.to_hex().dimmed());
    }

    Ok(())
}

/// Open editor for commit message editing
fn edit_commit_message(initial_message: &str) -> Result<String> {
    use std::io::{Read, Write};
    use std::process::Command;
    use tempfile::NamedTempFile;

    // Create temporary file with initial message
    let mut temp_file = NamedTempFile::new()?;
    writeln!(temp_file, "{}", initial_message)?;
    writeln!(temp_file)?;
    writeln!(
        temp_file,
        "# Please enter the commit message for your changes."
    )?;
    writeln!(temp_file, "# Lines starting with '#' will be ignored.")?;
    temp_file.flush()?;

    // Get editor from environment or use default
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    // Open editor
    let status = Command::new(&editor).arg(temp_file.path()).status()?;

    if !status.success() {
        return Err(anyhow::anyhow!("Editor exited with non-zero status"));
    }

    // Read back the edited message
    let mut edited_content = String::new();
    let mut file = std::fs::File::open(temp_file.path())?;
    file.read_to_string(&mut edited_content)?;

    // Filter out comment lines and trim
    let final_message = edited_content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if final_message.is_empty() {
        return Err(anyhow::anyhow!("Commit message cannot be empty"));
    }

    Ok(final_message)
}
