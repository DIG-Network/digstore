use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::json;

#[derive(Args)]
pub struct StoreInfoArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show configuration details
    #[arg(long)]
    pub config: bool,

    /// Show all paths
    #[arg(long)]
    pub paths: bool,
}

/// Execute the store-info command
pub fn execute(json: bool, config: bool, paths: bool) -> Result<()> {
    let args = StoreInfoArgs { json, config, paths };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    if args.json {
        show_store_info_json(&store, &args)?;
    } else {
        show_store_info_human(&store, &args)?;
    }

    Ok(())
}

fn show_store_info_human(store: &Store, args: &StoreInfoArgs) -> Result<()> {
    println!("{}", "Store Information".green().bold());
    println!("{}", "═".repeat(50).green());
    
    println!("{}: {}", "Store ID".bold(), store.store_id().to_hex().cyan());
    
    // Load Layer 0 metadata
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.layer");
    if layer_zero_path.exists() {
        let content = std::fs::read(layer_zero_path)?;
        let metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        if let Some(version) = metadata.get("digstore_version").and_then(|v| v.as_str()) {
            println!("{}: {}", "Digstore Version".bold(), version);
        }
        
        if let Some(format_version) = metadata.get("format_version").and_then(|v| v.as_str()) {
            println!("{}: {}", "Format Version".bold(), format_version);
        }
        
        if let Some(protocol_version) = metadata.get("protocol_version").and_then(|v| v.as_str()) {
            println!("{}: {}", "Protocol Version".bold(), protocol_version);
        }
        
        if let Some(created_at) = metadata.get("created_at").and_then(|v| v.as_i64()) {
            println!("{}: {}", "Created".bold(), format_timestamp(created_at));
        }
        
        if args.config {
            println!("\n{}", "Configuration:".bold());
            if let Some(config) = metadata.get("config") {
                if let Some(chunk_size) = config.get("chunk_size").and_then(|v| v.as_u64()) {
                    println!("  • Chunk size: {}", format_bytes(chunk_size));
                }
                if let Some(compression) = config.get("compression").and_then(|v| v.as_str()) {
                    println!("  • Compression: {}", compression);
                }
                if let Some(delta_limit) = config.get("delta_chain_limit").and_then(|v| v.as_u64()) {
                    println!("  • Delta chain limit: {}", delta_limit);
                }
            }
        }
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            println!("  • Commits: {}", root_history.len());
        }
    }
    
    if args.paths {
        println!("\n{}", "Paths:".bold());
        println!("  • Global Store: {}", store.global_path().display().to_string().cyan());
        
        if let Some(project_path) = store.project_path() {
            println!("  • Project Path: {}", project_path.display().to_string().cyan());
            println!("  • .layerstore File: {}", project_path.join(".layerstore").display().to_string().dimmed());
        }
    }
    
    // Show current status
    if let Some(current_root) = store.current_root() {
        println!("{}: {}", "Current Root".bold(), current_root.to_hex().cyan());
    } else {
        println!("{}: {}", "Current Root".bold(), "none".yellow());
    }
    
    // Calculate total size
    let total_size = std::fs::read_dir(store.global_path())?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    
    println!("{}: {}", "Total Size".bold(), format_bytes(total_size));
    
    // Count layer files
    let layer_count = std::fs::read_dir(store.global_path())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "dig")
                .unwrap_or(false)
        })
        .count();
    
    println!("{}: {}", "Layer Count".bold(), layer_count);

    Ok(())
}

fn show_store_info_json(store: &Store, args: &StoreInfoArgs) -> Result<()> {
    let mut store_info = json!({
        "store_id": store.store_id().to_hex(),
        "global_path": store.global_path().display().to_string(),
        "project_path": store.project_path().map(|p| p.display().to_string()),
        "current_root": store.current_root().map(|h| h.to_hex())
    });
    
    // Load Layer 0 metadata
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.layer");
    if layer_zero_path.exists() {
        let content = std::fs::read(layer_zero_path)?;
        let metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        store_info["digstore_version"] = metadata.get("digstore_version").cloned().unwrap_or(json!(null));
        store_info["format_version"] = metadata.get("format_version").cloned().unwrap_or(json!(null));
        store_info["protocol_version"] = metadata.get("protocol_version").cloned().unwrap_or(json!(null));
        store_info["created_at"] = metadata.get("created_at").cloned().unwrap_or(json!(null));
        
        if args.config {
            store_info["config"] = metadata.get("config").cloned().unwrap_or(json!({}));
        }
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            store_info["commit_count"] = json!(root_history.len());
        }
    }
    
    // Calculate total size
    let total_size = std::fs::read_dir(store.global_path())?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    
    store_info["total_size"] = json!(total_size);
    
    // Count layer files
    let layer_count = std::fs::read_dir(store.global_path())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "dig")
                .unwrap_or(false)
        })
        .count();
    
    store_info["layer_count"] = json!(layer_count);
    
    println!("{}", serde_json::to_string_pretty(&store_info)?);
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

    #[test]
    fn test_store_info_args() {
        let args = StoreInfoArgs {
            json: true,
            config: true,
            paths: true,
        };

        assert!(args.json);
        assert!(args.config);
        assert!(args.paths);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
    }
}
