use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::json;

#[derive(Args)]
pub struct LayersArgs {
    /// Layer hash to analyze (optional)
    pub layer_hash: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// List all layers
    #[arg(long)]
    pub list: bool,

    /// Show size information
    #[arg(long)]
    pub size: bool,

    /// Show file details
    #[arg(long)]
    pub files: bool,

    /// Show chunk details
    #[arg(long)]
    pub chunks: bool,
}

/// Layer information
#[derive(Debug, Clone, serde::Serialize)]
struct LayerInfo {
    hash: String,
    layer_type: String,
    generation: u64,
    parent_hash: String,
    timestamp: i64,
    file_size: u64,
    files_count: usize,
    chunks_count: usize,
    total_file_size: u64,
    commit_message: Option<String>,
    author: Option<String>,
}

/// Execute the layers command
pub fn execute(
    layer_hash: Option<String>,
    json: bool,
    list: bool,
    size: bool,
    files: bool,
    chunks: bool,
) -> Result<()> {
    let args = LayersArgs { layer_hash, json, list, size, files, chunks };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    if let Some(hash_str) = &args.layer_hash {
        // Analyze specific layer
        let layer_hash = crate::core::types::Hash::from_hex(hash_str)
            .map_err(|_| anyhow::anyhow!("Invalid layer hash: {}", hash_str))?;
        
        analyze_specific_layer(&store, layer_hash, &args)?;
    } else if args.list {
        // List all layers
        list_all_layers(&store, &args)?;
    } else {
        // Show current layer
        if let Some(current_root) = store.current_root() {
            analyze_specific_layer(&store, current_root, &args)?;
        } else {
            if args.json {
                println!("{}", json!({"error": "No current layer found"}));
            } else {
                println!("{}", "No current layer found".yellow());
                println!("  {} Use 'digstore commit' to create a layer", "→".cyan());
            }
        }
    }

    Ok(())
}

fn analyze_specific_layer(store: &Store, layer_hash: crate::core::types::Hash, args: &LayersArgs) -> Result<()> {
    let layer = store.load_layer(layer_hash)?;
    
    // Get layer file size from archive
    let file_size = if let Some(entry) = store.archive.list_layers().iter().find(|(hash, _)| *hash == layer_hash) {
        entry.1.size
    } else {
        0
    };
    
    let layer_info = LayerInfo {
        hash: layer_hash.to_hex(),
        layer_type: format!("{:?}", layer.header.get_layer_type().unwrap_or(crate::core::types::LayerType::Full)),
        generation: layer.header.layer_number,
        parent_hash: layer.header.get_parent_hash().to_hex(),
        timestamp: layer.header.timestamp as i64,
        file_size,
        files_count: layer.files.len(),
        chunks_count: layer.chunks.len(),
        total_file_size: layer.files.iter().map(|f| f.size).sum(),
        commit_message: layer.metadata.message.clone(),
        author: layer.metadata.author.clone(),
    };
    
    if args.json {
        show_layer_json(&layer_info, &layer, args)?;
    } else {
        show_layer_human(&layer_info, &layer, args)?;
    }
    
    Ok(())
}

fn list_all_layers(store: &Store, args: &LayersArgs) -> Result<()> {
    let mut layers = Vec::new();
    
    // Get all layers from archive
    let archive_layers = store.archive.list_layers();
    
    for (layer_hash, entry) in archive_layers {
        // Skip Layer 0 (metadata layer)
        if layer_hash == crate::core::types::Hash::zero() {
            continue;
        }
        
        // Load layer to get detailed information
        if let Ok(layer) = store.load_layer(layer_hash) {
            layers.push(LayerInfo {
                hash: layer_hash.to_hex(),
                layer_type: format!("{:?}", layer.header.get_layer_type().unwrap_or(crate::core::types::LayerType::Full)),
                generation: layer.header.layer_number,
                parent_hash: layer.header.get_parent_hash().to_hex(),
                timestamp: layer.header.timestamp as i64,
                file_size: entry.size,
                files_count: layer.files.len(),
                chunks_count: layer.chunks.len(),
                total_file_size: layer.files.iter().map(|f| f.size).sum(),
                commit_message: layer.metadata.message.clone(),
                author: layer.metadata.author.clone(),
            });
        }
    }
    
    // Sort by generation
    layers.sort_by_key(|l| l.generation);
    
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({
            "layers": layers,
            "total_layers": layers.len()
        }))?);
    } else {
        println!("{}", "Layer List".green().bold());
        println!("{}", "═".repeat(50).green());
        
        if layers.is_empty() {
            println!("{}", "No layers found".yellow());
            return Ok(());
        }
        
        for layer in &layers {
            println!("\n{} {}", "Layer".bold(), layer.hash[..16].cyan());
            println!("  Type: {}", layer.layer_type);
            println!("  Generation: {}", layer.generation);
            println!("  Size: {}", format_bytes(layer.file_size));
            println!("  Files: {}", layer.files_count);
            
            if let Some(message) = &layer.commit_message {
                println!("  Message: {}", message.bright_white());
            }
        }
        
        println!("\n{}", format!("Total layers: {}", layers.len()).cyan());
    }
    
    Ok(())
}

fn show_layer_human(layer_info: &LayerInfo, layer: &crate::storage::layer::Layer, args: &LayersArgs) -> Result<()> {
    println!("{}", "Layer Analysis".green().bold());
    println!("{}", "═".repeat(50).green());
    
    println!("{}: {}", "Layer Hash".bold(), layer_info.hash.cyan());
    println!("{}: {}", "Type".bold(), layer_info.layer_type);
    println!("{}: {}", "Generation".bold(), layer_info.generation);
    println!("{}: {}", "Parent Hash".bold(), layer_info.parent_hash.cyan());
    println!("{}: {}", "Timestamp".bold(), format_timestamp(layer_info.timestamp));
    
    if args.size {
        println!("\n{}", "Storage Details:".bold());
        println!("  Layer File Size: {}", format_bytes(layer_info.file_size));
        println!("  Uncompressed Size: {}", format_bytes(layer_info.total_file_size));
        if layer_info.total_file_size > 0 {
            let compression_ratio = 1.0 - (layer_info.file_size as f64 / layer_info.total_file_size as f64);
            println!("  Compression Ratio: {:.1}%", compression_ratio * 100.0);
        }
        println!("  Scrambling: Enabled (URN-protected)");
    }
    
    println!("\n{}", "Content Summary:".bold());
    println!("  Files: {}", layer_info.files_count);
    println!("  Total File Size: {}", format_bytes(layer_info.total_file_size));
    println!("  Chunks: {}", layer_info.chunks_count);
    
    if layer_info.chunks_count > 0 {
        println!("  Average Chunk Size: {}", format_bytes(layer_info.total_file_size / layer_info.chunks_count as u64));
    }
    
    if let Some(message) = &layer_info.commit_message {
        println!("  Commit Message: {}", message.bright_white());
    }
    
    if let Some(author) = &layer_info.author {
        println!("  Author: {}", author);
    }
    
    if args.files && !layer.files.is_empty() {
        println!("\n{}", "Files:".bold());
        for (i, file) in layer.files.iter().enumerate().take(10) {
            println!("  {}. {} ({}, {} chunks)", 
                     i + 1,
                     file.path.display().to_string().cyan(),
                     format_bytes(file.size),
                     file.chunks.len());
        }
        
        if layer.files.len() > 10 {
            println!("  ... and {} more files", layer.files.len() - 10);
        }
    }
    
    if args.chunks && !layer.chunks.is_empty() {
        println!("\n{}", "Chunk Analysis:".bold());
        
        // Analyze chunk size distribution
        let mut size_buckets = [0; 4]; // <1KB, 1KB-32KB, 32KB-1MB, >1MB
        for chunk in &layer.chunks {
            let size = chunk.size;
            if size < 1024 {
                size_buckets[0] += 1;
            } else if size < 32768 {
                size_buckets[1] += 1;
            } else if size < 1048576 {
                size_buckets[2] += 1;
            } else {
                size_buckets[3] += 1;
            }
        }
        
        println!("  Size Distribution:");
        println!("    < 1KB: {} chunks ({:.1}%)", size_buckets[0], size_buckets[0] as f64 / layer.chunks.len() as f64 * 100.0);
        println!("    1KB-32KB: {} chunks ({:.1}%)", size_buckets[1], size_buckets[1] as f64 / layer.chunks.len() as f64 * 100.0);
        println!("    32KB-1MB: {} chunks ({:.1}%)", size_buckets[2], size_buckets[2] as f64 / layer.chunks.len() as f64 * 100.0);
        println!("    > 1MB: {} chunks ({:.1}%)", size_buckets[3], size_buckets[3] as f64 / layer.chunks.len() as f64 * 100.0);
    }

    Ok(())
}

fn show_layer_json(layer_info: &LayerInfo, layer: &crate::storage::layer::Layer, args: &LayersArgs) -> Result<()> {
    let mut output = json!(layer_info);
    
    if args.files {
        output["files"] = json!(layer.files.iter().map(|f| json!({
            "path": f.path.display().to_string(),
            "hash": f.hash.to_hex(),
            "size": f.size,
            "chunks": f.chunks.len()
        })).collect::<Vec<_>>());
    }
    
    if args.chunks {
        // Analyze chunk size distribution
        let mut size_buckets = [0; 4];
        for chunk in &layer.chunks {
            let size = chunk.size;
            if size < 1024 {
                size_buckets[0] += 1;
            } else if size < 32768 {
                size_buckets[1] += 1;
            } else if size < 1048576 {
                size_buckets[2] += 1;
            } else {
                size_buckets[3] += 1;
            }
        }
        
        output["chunk_analysis"] = json!({
            "total_chunks": layer.chunks.len(),
            "size_distribution": {
                "under_1kb": size_buckets[0],
                "1kb_to_32kb": size_buckets[1],
                "32kb_to_1mb": size_buckets[2],
                "over_1mb": size_buckets[3]
            }
        });
    }
    
    println!("{}", serde_json::to_string_pretty(&output)?);
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
    fn test_layers_args() {
        let args = LayersArgs {
            layer_hash: Some("abc123".to_string()),
            json: true,
            list: true,
            size: true,
            files: true,
            chunks: true,
        };

        assert_eq!(args.layer_hash, Some("abc123".to_string()));
        assert!(args.json);
        assert!(args.list);
        assert!(args.size);
        assert!(args.files);
        assert!(args.chunks);
    }
}
