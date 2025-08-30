//! Add command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::{find_repository_root};
use std::path::PathBuf;
use colored::Colorize;

/// Execute the add command
pub fn execute(
    paths: Vec<PathBuf>,
    recursive: bool,
    all: bool,
    force: bool,
    dry_run: bool,
    from_stdin: bool,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let mut store = Store::open(&repo_root)?;

    if dry_run {
        println!("{}", "Files that would be added:".bright_blue());
    } else {
        println!("{}", "Adding files to staging...".bright_blue());
    }

    let mut files_added = 0;
    let mut total_size = 0u64;

    if all {
        // Add all files in repository
        println!("  {} Adding all files in repository...", "•".cyan());
        if !dry_run {
            store.add_directory(&repo_root, true)?;
        }
        let status = store.status();
        files_added = status.staged_files.len();
        total_size = status.total_staged_size;
    } else if from_stdin {
        // Read file list from stdin
        println!("  {} Reading file list from stdin...", "•".cyan());
        use std::io::{self, BufRead};
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let file_path = PathBuf::from(line?);
            if !dry_run {
                store.add_file(&file_path)?;
            }
            println!("    {} {}", "•".green(), file_path.display());
            files_added += 1;
        }
    } else {
        // Add specified paths
        for path in &paths {
            if path.is_dir() && recursive {
                println!("  {} Adding directory: {} (recursive)", "•".cyan(), path.display());
                if !dry_run {
                    store.add_directory(path, true)?;
                }
            } else if path.is_dir() && !recursive {
                println!("  {} Skipping directory: {} (use -r for recursive)", "!".yellow(), path.display());
                continue;
            } else if path.is_file() {
                println!("  {} Adding file: {}", "•".cyan(), path.display());
                if !dry_run {
                    store.add_file(path)?;
                }
                files_added += 1;
                if let Ok(metadata) = std::fs::metadata(path) {
                    total_size += metadata.len();
                }
            } else {
                println!("  {} File not found: {}", "✗".red(), path.display());
                if !force {
                    return Err(anyhow::anyhow!("File not found: {}", path.display()));
                }
            }
        }
    }

    // Get final status
    let final_status = store.status();
    files_added = final_status.staged_files.len();
    total_size = final_status.total_staged_size;

    println!();
    if dry_run {
        println!("{} {} files would be added ({} bytes)", 
            "Would add:".bright_green().bold(), 
            files_added, 
            total_size);
    } else {
        println!("{} {} files added to staging ({} bytes)", 
            "✓".green().bold(), 
            files_added, 
            total_size);
        
        if files_added > 0 {
            println!("  {} Use 'digstore commit -m \"message\"' to create a commit", "→".cyan());
        }
    }

    Ok(())
}
