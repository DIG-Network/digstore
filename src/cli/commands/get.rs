//! Get command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::find_repository_root;
use crate::urn::{Urn, parse_urn};
use crate::core::types::Hash;
use std::path::PathBuf;
use std::io::Write;
use colored::Colorize;

/// Execute the get command
pub fn execute(
    path: String,
    output: Option<PathBuf>,
    verify: bool,
    metadata: bool,
    at: Option<String>,
    progress: bool,
) -> Result<()> {
    println!("{}", "Retrieving content...".bright_blue());

    // Parse the at parameter if provided
    let at_root = if let Some(hash_str) = at {
        Some(Hash::from_hex(&hash_str)
            .map_err(|_| anyhow::anyhow!("Invalid root hash: {}", hash_str))?)
    } else {
        None
    };

    let content = if path.starts_with("urn:dig:chia:") {
        // Full URN provided - parse and resolve
        println!("  {} Parsing URN: {}", "•".cyan(), path.dimmed());
        let urn = parse_urn(&path)?;
        
        // For URN resolution, we need to open the store by ID
        let store = Store::open_global(&urn.store_id)?;
        urn.resolve(&store)?
    } else {
        // Simple path - find repository and resolve
        let repo_root = find_repository_root()?
            .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

        let store = Store::open(&repo_root)?;
        let file_path = PathBuf::from(&path);
        
        println!("  {} Retrieving file: {}", "•".cyan(), file_path.display());
        
        if let Some(root_hash) = at_root {
            store.get_file_at(&file_path, Some(root_hash))?
        } else {
            store.get_file(&file_path)?
        }
    };

    if progress {
        println!("  {} Retrieved {} bytes", "✓".green(), content.len());
    }

    // Handle output
    if let Some(output_path) = output {
        // Write to file
        std::fs::write(&output_path, &content)?;
        println!("{} Content written to: {}", 
            "✓".green().bold(), 
            output_path.display().to_string().bright_white());
        
        if metadata {
            println!("  {} Size: {} bytes", "→".cyan(), content.len());
            if verify {
                println!("  {} Content verified", "✓".green());
            }
        }
    } else {
        // Write to stdout
        std::io::stdout().write_all(&content)?;
        
        if metadata {
            eprintln!("{} Size: {} bytes", "→".cyan(), content.len());
            if verify {
                eprintln!("  {} Content verified", "✓".green());
            }
        }
    }

    Ok(())
}
