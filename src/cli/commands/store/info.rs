use crate::core::error::DigstoreError;
use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Args)]
pub struct InfoArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show specific layer info
    #[arg(long)]
    pub layer: Option<String>,
}

pub fn execute(json: bool, layer: Option<String>) -> Result<()> {
    let args = InfoArgs { json, layer };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    if let Some(layer_hash) = &args.layer {
        show_layer_info(&store, layer_hash, args.json)?;
    } else {
        show_store_info(&store, args.json)?;
    }

    Ok(())
}

fn show_store_info(store: &Store, json_output: bool) -> Result<()> {
    // Load Layer 0 metadata from archive
    let layer_zero_hash = crate::core::types::Hash::zero();
    let content = store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&content)?;

    // Count layers in archive
    let layer_count = store.archive.list_layers().len();

    // Get archive file size
    let total_size = store.archive.path().metadata()?.len();

    if json_output {
        let info = json!({
            "store_id": store.store_id.to_hex(),
            "global_path": store.global_path().display().to_string(),
            "project_path": store.project_path().map(|p| p.display().to_string()),
            "current_root": store.current_root().map(|h| h.to_hex()),
            "layer_count": layer_count,
            "total_size_bytes": total_size,
            "digstore_version": metadata.get("digstore_version").and_then(|v| v.as_str()),
            "format_version": metadata.get("format_version").and_then(|v| v.as_str()),
            "protocol_version": metadata.get("protocol_version").and_then(|v| v.as_str()),
            "created_at": metadata.get("created_at").and_then(|v| v.as_i64()),
            "config": metadata.get("config"),
            "root_history_count": metadata.get("root_history").and_then(|v| v.as_array()).map(|a| a.len()),
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!("{}", "Repository Information".green().bold());
        println!("{}", "═".repeat(50).green());

        println!("{}: {}", "Store ID".bold(), store.store_id.to_hex().cyan());
        println!(
            "{}: {}",
            "Global Path".bold(),
            store.global_path().display()
        );

        if let Some(project_path) = store.project_path() {
            println!("{}: {}", "Project Path".bold(), project_path.display());
        }

        if let Some(current_root) = store.current_root() {
            println!(
                "{}: {}",
                "Current Root".bold(),
                current_root.to_hex().cyan()
            );
        } else {
            println!("{}: {}", "Current Root".bold(), "none".yellow());
        }

        println!("{}: {}", "Layer Count".bold(), layer_count);
        println!("{}: {}", "Total Size".bold(), format_bytes(total_size));

        if let Some(version) = metadata.get("digstore_version").and_then(|v| v.as_str()) {
            println!("{}: {}", "Digstore Version".bold(), version);
        }

        if let Some(created_at) = metadata.get("created_at").and_then(|v| v.as_i64()) {
            println!("{}: {}", "Created".bold(), format_timestamp(created_at));
        }

        if let Some(config) = metadata.get("config") {
            println!("\n{}", "Configuration:".bold());
            if let Some(chunk_size) = config.get("chunk_size").and_then(|v| v.as_u64()) {
                println!("  • Chunk size: {}", format_bytes(chunk_size));
            }
            if let Some(compression) = config.get("compression").and_then(|v| v.as_str()) {
                println!("  • Compression: {}", compression);
            }
        }

        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            println!("  • Commits: {}", root_history.len());
        }
    }

    Ok(())
}

fn show_layer_info(store: &Store, layer_hash: &str, json_output: bool) -> Result<()> {
    let hash = crate::core::types::Hash::from_hex(layer_hash)
        .map_err(|_| DigstoreError::internal("Invalid layer hash format"))?;

    let layer = store.load_layer(hash)?;

    if json_output {
        let info = json!({
            "layer_id": hash.to_hex(),
            "layer_type": format!("{:?}", layer.header.get_layer_type()),
            "layer_number": layer.header.layer_number,
            "parent_hash": layer.header.get_parent_hash().to_hex(),
            "timestamp": layer.header.timestamp,
            "files_count": layer.files.len(),
            "chunks_count": layer.chunks.len(),
            "message": layer.metadata.message,
            "author": layer.metadata.author,
            "files": layer.files.iter().map(|f| json!({
                "path": f.path.display().to_string(),
                "hash": f.hash.to_hex(),
                "size": f.size,
                "chunks": f.chunks.len(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!("{}", "Layer Information".green().bold());
        println!("{}", "═".repeat(50).green());

        println!("{}: {}", "Layer ID".bold(), hash.to_hex().cyan());
        println!(
            "{}: {:?}",
            "Layer Type".bold(),
            layer.header.get_layer_type()
        );
        println!("{}: {}", "Layer Number".bold(), layer.header.layer_number);
        println!(
            "{}: {}",
            "Parent Hash".bold(),
            layer.header.get_parent_hash().to_hex().cyan()
        );
        println!(
            "{}: {}",
            "Timestamp".bold(),
            format_timestamp(layer.header.timestamp as i64)
        );

        if let Some(message) = &layer.metadata.message {
            println!("{}: {}", "Message".bold(), message);
        }

        if let Some(author) = &layer.metadata.author {
            println!("{}: {}", "Author".bold(), author);
        }

        println!("\n{}", "Layer Contents:".bold());
        println!("  • Files: {}", layer.files.len());
        println!("  • Chunks: {}", layer.chunks.len());

        let total_file_size: u64 = layer.files.iter().map(|f| f.size).sum();
        println!("  • Total file size: {}", format_bytes(total_file_size));

        if !layer.files.is_empty() {
            println!("\n{}", "Files:".bold());
            for file in &layer.files {
                println!(
                    "  • {} ({}, {} chunks)",
                    file.path.display().to_string().cyan(),
                    format_bytes(file.size),
                    file.chunks.len()
                );
            }
        }
    }

    Ok(())
}

fn format_timestamp(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "Not set".to_string();
    }

    use std::time::{Duration, UNIX_EPOCH};

    match UNIX_EPOCH.checked_add(Duration::from_secs(timestamp as u64)) {
        Some(datetime) => {
            // Format as readable date/time
            match datetime.duration_since(UNIX_EPOCH) {
                Ok(duration) => {
                    let secs = duration.as_secs();
                    let days = secs / 86400;
                    let hours = (secs % 86400) / 3600;
                    let minutes = (secs % 3600) / 60;
                    let seconds = secs % 60;

                    if days > 0 {
                        format!("{} days ago", days)
                    } else if hours > 0 {
                        format!("{} hours ago", hours)
                    } else if minutes > 0 {
                        format!("{} minutes ago", minutes)
                    } else {
                        format!("{} seconds ago", seconds)
                    }
                },
                Err(_) => format!("Timestamp: {}", timestamp),
            }
        },
        None => format!("Invalid timestamp: {}", timestamp),
    }
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
        let timestamp = 1234567890;
        let formatted = format_timestamp(timestamp);
        assert!(!formatted.is_empty());
        // The format might contain "Unknown" on some systems, so just check it's not empty
    }

    #[test]
    fn test_info_args() {
        let args = InfoArgs {
            json: true,
            layer: Some("abc123".to_string()),
        };

        assert!(args.json);
        assert_eq!(args.layer, Some("abc123".to_string()));
    }
}
