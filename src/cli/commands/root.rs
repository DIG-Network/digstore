use crate::core::error::DigstoreError;
use crate::storage::Store;
use crate::cli::commands::find_repository_root;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::json;

#[derive(Args)]
pub struct RootArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show detailed information
    #[arg(short, long)]
    pub verbose: bool,

    /// Show only the root hash
    #[arg(long)]
    pub hash_only: bool,
}

/// Execute the root command
pub fn execute(json: bool, verbose: bool, hash_only: bool) -> Result<()> {
    let args = RootArgs { json, verbose, hash_only };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    if let Some(current_root) = store.current_root() {
        if args.hash_only {
            println!("{}", current_root.to_hex());
            return Ok(());
        }

        if args.json {
            show_root_json(&store, current_root, args.verbose)?;
        } else {
            show_root_human(&store, current_root, args.verbose)?;
        }
    } else {
        if args.json {
            println!("{}", json!({"error": "No commits found", "current_root": null}));
        } else {
            println!("{}", "No commits found".yellow());
            println!("  {} Use 'digstore commit' to create the first commit", "→".cyan());
        }
    }

    Ok(())
}

fn show_root_human(store: &Store, root_hash: crate::core::types::Hash, verbose: bool) -> Result<()> {
    println!("{}", "Current Root Information".green().bold());
    println!("{}", "═".repeat(50).green());
    
    println!("{}: {}", "Root Hash".bold(), root_hash.to_hex().cyan());
    
    // Load Layer 0 to get generation info
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.dig");
    if layer_zero_path.exists() {
        let content = std::fs::read(layer_zero_path)?;
        let metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            if let Some(current_entry) = root_history.iter().find(|entry| {
                entry.get("root_hash").and_then(|h| h.as_str()) == Some(&root_hash.to_hex())
            }) {
                if let Some(generation) = current_entry.get("generation").and_then(|g| g.as_u64()) {
                    println!("{}: {}", "Generation".bold(), generation);
                }
                
                if let Some(timestamp) = current_entry.get("timestamp").and_then(|t| t.as_i64()) {
                    println!("{}: {}", "Timestamp".bold(), format_timestamp(timestamp));
                }
                
                if let Some(layer_count) = current_entry.get("layer_count").and_then(|c| c.as_u64()) {
                    println!("{}: {}", "Layer Count".bold(), layer_count);
                }
            }
        }
    }
    
    // Get layer file information
    let layer_path = store.global_path().join(format!("{}.dig", root_hash.to_hex()));
    if layer_path.exists() {
        if let Ok(metadata) = std::fs::metadata(&layer_path) {
            println!("{}: {}", "Layer File Size".bold(), format_bytes(metadata.len()));
        }
        
        if verbose {
            // Load layer to get detailed information
            if let Ok(layer) = store.load_layer(root_hash) {
                println!();
                println!("{}", "Layer Details:".bold());
                println!("  • Files: {}", layer.files.len());
                println!("  • Chunks: {}", layer.chunks.len());
                
                let total_file_size: u64 = layer.files.iter().map(|f| f.size).sum();
                println!("  • Total File Size: {}", format_bytes(total_file_size));
                
                if let Some(message) = &layer.metadata.message {
                    println!("  • Commit Message: {}", message.bright_white());
                }
                
                if let Some(author) = &layer.metadata.author {
                    println!("  • Author: {}", author);
                }
            }
        }
    }
    
    println!();
    println!("{}: {}", "Layer File".bold(), format!("{}.dig", root_hash.to_hex()).dimmed());

    Ok(())
}

fn show_root_json(store: &Store, root_hash: crate::core::types::Hash, verbose: bool) -> Result<()> {
    let mut root_info = json!({
        "root_hash": root_hash.to_hex(),
        "generation": null,
        "timestamp": null,
        "layer_count": null,
        "layer_file_size": null,
        "layer_file_path": format!("{}.dig", root_hash.to_hex())
    });
    
    // Load Layer 0 for generation info
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.dig");
    if layer_zero_path.exists() {
        let content = std::fs::read(layer_zero_path)?;
        let metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            if let Some(current_entry) = root_history.iter().find(|entry| {
                entry.get("root_hash").and_then(|h| h.as_str()) == Some(&root_hash.to_hex())
            }) {
                root_info["generation"] = current_entry.get("generation").cloned().unwrap_or(json!(null));
                root_info["timestamp"] = current_entry.get("timestamp").cloned().unwrap_or(json!(null));
                root_info["layer_count"] = current_entry.get("layer_count").cloned().unwrap_or(json!(null));
            }
        }
    }
    
    // Get layer file size
    let layer_path = store.global_path().join(format!("{}.dig", root_hash.to_hex()));
    if layer_path.exists() {
        if let Ok(metadata) = std::fs::metadata(&layer_path) {
            root_info["layer_file_size"] = json!(metadata.len());
        }
        
        if verbose {
            // Load layer for detailed information
            if let Ok(layer) = store.load_layer(root_hash) {
                root_info["layer_details"] = json!({
                    "files_count": layer.files.len(),
                    "chunks_count": layer.chunks.len(),
                    "total_file_size": layer.files.iter().map(|f| f.size).sum::<u64>(),
                    "commit_message": layer.metadata.message,
                    "author": layer.metadata.author,
                    "layer_type": format!("{:?}", layer.header.get_layer_type()),
                    "layer_number": layer.header.layer_number
                });
            }
        }
    }
    
    println!("{}", serde_json::to_string_pretty(&root_info)?);
    Ok(())
}

fn format_timestamp(timestamp: i64) -> String {
    use std::time::{UNIX_EPOCH, Duration};
    
    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    format!("{:?}", datetime).split_once('.').map(|(s, _)| s).unwrap_or("Unknown").to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_root_command_no_repository() {
        let result = execute(false, false, false);
        assert!(result.is_err()); // Should fail when no repository
    }

    #[test]
    fn test_root_command_args() {
        let args = RootArgs {
            json: true,
            verbose: true,
            hash_only: false,
        };

        assert!(args.json);
        assert!(args.verbose);
        assert!(!args.hash_only);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_format_timestamp() {
        let timestamp = 1693422642; // Example timestamp
        let formatted = format_timestamp(timestamp);
        assert!(!formatted.is_empty());
        assert!(!formatted.contains("Unknown"));
    }
}
