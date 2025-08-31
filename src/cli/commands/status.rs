//! Status command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::find_repository_root;
use colored::Colorize;
use tabled::{Table, Tabled};

/// Execute the status command
pub fn execute(
    short: bool,
    porcelain: bool,
    show_chunks: bool,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .layerstore file found)"))?;

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
        
        if show_chunks {
            // Enhanced table view with chunk information
            #[derive(Tabled)]
            struct FileStatus {
                #[tabled(rename = "Status")]
                status: String,
                #[tabled(rename = "File")]
                file: String,
                #[tabled(rename = "Size")]
                size: String,
                #[tabled(rename = "Chunks")]
                chunks: String,
            }
            
            let mut file_statuses = Vec::new();
            for file_path in &status.staged_files {
                // Try to get file size
                let size = std::fs::metadata(file_path)
                    .map(|m| format_bytes(m.len()))
                    .unwrap_or_else(|_| "unknown".to_string());
                
                file_statuses.push(FileStatus {
                    status: "new file".to_string(),
                    file: file_path.display().to_string(),
                    size,
                    chunks: "pending".to_string(), // Would calculate during actual chunking
                });
            }
            
            let table = Table::new(file_statuses);
            println!("{}", table);
        } else {
            // Simple list view
            for file_path in &status.staged_files {
                println!("  {} {}", "new file:".green(), file_path.display());
            }
        }
        
        println!();
        println!("Summary:");
        println!("  Files staged: {}", status.staged_files.len());
        println!("  Total size: {} bytes", status.total_staged_size);
    } else {
        println!("{}", "No changes staged for commit".dimmed());
        println!("  {} Use 'digstore add <files>' to stage files", "→".cyan());
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}
