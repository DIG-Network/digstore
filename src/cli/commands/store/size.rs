use crate::cli::commands::find_repository_root;
use crate::core::error::DigstoreError;
use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::json;
use std::collections::HashMap;

#[derive(Args)]
pub struct SizeArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show detailed breakdown
    #[arg(long)]
    pub breakdown: bool,

    /// Show deduplication metrics
    #[arg(long)]
    pub efficiency: bool,

    /// Show per-layer analysis
    #[arg(long)]
    pub layers: bool,
}

/// Execute the size command
pub fn execute(json: bool, breakdown: bool, efficiency: bool, layers: bool) -> Result<()> {
    let args = SizeArgs {
        json,
        breakdown,
        efficiency,
        layers,
    };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    let storage_analysis = analyze_storage(&store)?;

    if args.json {
        show_size_json(&storage_analysis, &args)?;
    } else {
        show_size_human(&storage_analysis, &args)?;
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
struct StorageAnalysis {
    total_size: u64,
    layer_files_size: u64,
    metadata_size: u64,
    staging_size: u64,
    layer_count: usize,
    layer_breakdown: Vec<LayerInfo>,
    efficiency_metrics: EfficiencyMetrics,
}

#[derive(Debug, Clone, serde::Serialize)]
struct LayerInfo {
    hash: String,
    size: u64,
    files_count: usize,
    chunks_count: usize,
    total_file_size: u64,
    layer_type: String,
    generation: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct EfficiencyMetrics {
    deduplication_ratio: f64,
    compression_ratio: f64,
    storage_efficiency: f64,
    unique_chunks: usize,
    total_chunks: usize,
    bytes_saved: u64,
}

fn analyze_storage(store: &Store) -> Result<StorageAnalysis> {
    let mut total_size = 0u64;
    let mut layer_files_size = 0u64;
    let mut layer_breakdown = Vec::new();
    let mut all_chunks = HashMap::new();
    let mut total_file_size = 0u64;

    // Analyze all layers in archive
    let archive_layers = store.archive.list_layers();
    total_size = store.archive.path().metadata()?.len();
    layer_files_size = total_size;

    for (layer_hash, entry) in archive_layers {
        // Skip Layer 0 for layer analysis
        if layer_hash != crate::core::types::Hash::zero() {
            if let Ok(layer) = store.load_layer(layer_hash) {
                let layer_total_file_size: u64 = layer.files.iter().map(|f| f.size).sum();
                total_file_size += layer_total_file_size;

                // Collect chunks for deduplication analysis
                for chunk in &layer.chunks {
                    *all_chunks.entry(chunk.hash).or_insert(0) += 1;
                }

                layer_breakdown.push(LayerInfo {
                    hash: layer_hash.to_hex(),
                    size: entry.size,
                    files_count: layer.files.len(),
                    chunks_count: layer.chunks.len(),
                    total_file_size: layer_total_file_size,
                    layer_type: format!(
                        "{:?}",
                        layer
                            .header
                            .get_layer_type()
                            .unwrap_or(crate::core::types::LayerType::Full)
                    ),
                    generation: layer.header.layer_number,
                });
            }
        }
    }

    // Calculate staging size (binary staging format)
    let staging_path = store.archive.path().with_extension("staging.bin");
    let staging_size = if staging_path.exists() {
        std::fs::metadata(staging_path)?.len()
    } else {
        0
    };
    total_size += staging_size;

    // Calculate efficiency metrics
    let total_chunks = all_chunks.values().sum::<usize>();
    let unique_chunks = all_chunks.len();
    let duplicate_chunks = total_chunks.saturating_sub(unique_chunks);
    let deduplication_ratio = if total_chunks > 0 {
        duplicate_chunks as f64 / total_chunks as f64
    } else {
        0.0
    };

    // Estimate compression ratio (simplified)
    let compression_ratio = if total_file_size > 0 && layer_files_size > 0 {
        1.0 - (layer_files_size as f64 / total_file_size as f64)
    } else {
        0.0
    };

    let storage_efficiency =
        deduplication_ratio + compression_ratio - (deduplication_ratio * compression_ratio);
    let bytes_saved = (total_file_size as f64 * storage_efficiency) as u64;

    let efficiency_metrics = EfficiencyMetrics {
        deduplication_ratio,
        compression_ratio,
        storage_efficiency,
        unique_chunks,
        total_chunks,
        bytes_saved,
    };

    Ok(StorageAnalysis {
        total_size,
        layer_files_size,
        metadata_size: layer_files_size - layer_breakdown.iter().map(|l| l.size).sum::<u64>(),
        staging_size,
        layer_count: layer_breakdown.len(),
        layer_breakdown,
        efficiency_metrics,
    })
}

fn show_size_human(analysis: &StorageAnalysis, args: &SizeArgs) -> Result<()> {
    println!("{}", "Storage Analytics".green().bold());
    println!("{}", "═".repeat(50).green());

    println!(
        "{}: {}",
        "Total Storage".bold(),
        format_bytes(analysis.total_size)
    );

    if args.breakdown {
        println!(
            "├─ Layer Files: {} ({:.1}%)",
            format_bytes(analysis.layer_files_size),
            analysis.layer_files_size as f64 / analysis.total_size as f64 * 100.0
        );
        println!(
            "├─ Metadata: {} ({:.1}%)",
            format_bytes(analysis.metadata_size),
            analysis.metadata_size as f64 / analysis.total_size as f64 * 100.0
        );
        println!(
            "├─ Staging: {} ({:.1}%)",
            format_bytes(analysis.staging_size),
            analysis.staging_size as f64 / analysis.total_size as f64 * 100.0
        );
        println!("└─ Overhead: {} ({:.1}%)", format_bytes(0), 0.0);
    }

    if args.layers && !analysis.layer_breakdown.is_empty() {
        println!("\n{}", "Layer Breakdown:".bold());
        for (i, layer) in analysis.layer_breakdown.iter().enumerate() {
            println!(
                "  Layer {} ({}): {} - {} files, {} chunks",
                i + 1,
                layer.layer_type,
                format_bytes(layer.size),
                layer.files_count,
                layer.chunks_count
            );
        }

        let avg_layer_size = analysis.layer_files_size / analysis.layer_count.max(1) as u64;
        println!("  Average layer size: {}", format_bytes(avg_layer_size));
    }

    if args.efficiency {
        println!("\n{}", "Efficiency Metrics:".bold());
        println!(
            "  Deduplication Ratio: {:.1}% ({} saved)",
            analysis.efficiency_metrics.deduplication_ratio * 100.0,
            format_bytes(analysis.efficiency_metrics.bytes_saved)
        );
        println!(
            "  Compression Ratio: {:.1}%",
            analysis.efficiency_metrics.compression_ratio * 100.0
        );
        println!(
            "  Storage Efficiency: {:.1}%",
            analysis.efficiency_metrics.storage_efficiency * 100.0
        );
        println!(
            "  Unique Chunks: {} of {} ({:.1}%)",
            analysis.efficiency_metrics.unique_chunks,
            analysis.efficiency_metrics.total_chunks,
            analysis.efficiency_metrics.unique_chunks as f64
                / analysis.efficiency_metrics.total_chunks.max(1) as f64
                * 100.0
        );
    }

    Ok(())
}

fn show_size_json(analysis: &StorageAnalysis, args: &SizeArgs) -> Result<()> {
    let mut output = json!({
        "total_size": analysis.total_size,
        "layer_files_size": analysis.layer_files_size,
        "metadata_size": analysis.metadata_size,
        "staging_size": analysis.staging_size,
        "layer_count": analysis.layer_count
    });

    if args.breakdown {
        output["breakdown"] = json!({
            "layer_files": {
                "size": analysis.layer_files_size,
                "percentage": analysis.layer_files_size as f64 / analysis.total_size as f64 * 100.0
            },
            "metadata": {
                "size": analysis.metadata_size,
                "percentage": analysis.metadata_size as f64 / analysis.total_size as f64 * 100.0
            },
            "staging": {
                "size": analysis.staging_size,
                "percentage": analysis.staging_size as f64 / analysis.total_size as f64 * 100.0
            }
        });
    }

    if args.layers {
        output["layers"] = json!(analysis.layer_breakdown);
    }

    if args.efficiency {
        output["efficiency"] = json!(analysis.efficiency_metrics);
    }

    println!("{}", serde_json::to_string_pretty(&output)?);
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
    fn test_size_args() {
        let args = SizeArgs {
            json: true,
            breakdown: true,
            efficiency: true,
            layers: false,
        };

        assert!(args.json);
        assert!(args.breakdown);
        assert!(args.efficiency);
        assert!(!args.layers);
    }
}
