//! Get command implementation

use crate::cli::commands::find_repository_root;
use crate::core::types::Hash;
use crate::storage::store::Store;
use crate::urn::{parse_urn, Urn};
use anyhow::Result;
use base64;
use colored::Colorize;
use std::io::Write;
use std::path::PathBuf;

/// Execute the get command
pub fn execute(
    path: String,
    output: Option<PathBuf>,
    verify: bool,
    metadata: bool,
    at: Option<String>,
    progress: bool,
    json: bool,
) -> Result<()> {
    println!("{}", "Retrieving content...".bright_blue());

    // Parse the at parameter if provided
    let at_root = if let Some(hash_str) = at {
        Some(
            Hash::from_hex(&hash_str)
                .map_err(|_| anyhow::anyhow!("Invalid root hash: {}", hash_str))?,
        )
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
    if let Some(output_path) = &output {
        // Write to file (-o flag)
        std::fs::write(output_path, &content)?;

        if json {
            // JSON metadata about the file operation
            let output_info = serde_json::json!({
                "action": "file_written",
                "path": path,
                "output_file": output_path.display().to_string(),
                "size": content.len(),
                "verified": verify,
                "at_root": at_root.map(|h| h.to_hex()),
                "metadata_included": metadata
            });
            println!("{}", serde_json::to_string_pretty(&output_info)?);
        } else {
            println!(
                "{} Content written to: {}",
                "✓".green().bold(),
                output_path.display().to_string().bright_white()
            );

            if metadata {
                println!("  {} Size: {} bytes", "→".cyan(), content.len());
                if verify {
                    println!("  {} Content verified", "✓".green());
                }
            }
        }
    } else {
        // Stream to stdout (default behavior)
        if json {
            // JSON metadata to stderr, content to stdout
            let output_info = serde_json::json!({
                "action": "content_streamed",
                "path": path,
                "size": content.len(),
                "verified": verify,
                "at_root": at_root.map(|h| h.to_hex()),
                "metadata_included": metadata
            });
            eprintln!("{}", serde_json::to_string_pretty(&output_info)?);
        }

        // Always stream content to stdout
        std::io::stdout().write_all(&content)?;

        if metadata && !json {
            eprintln!("{} Size: {} bytes", "→".cyan(), content.len());
            if verify {
                eprintln!("  {} Content verified", "✓".green());
            }
        }
    }

    Ok(())
}
