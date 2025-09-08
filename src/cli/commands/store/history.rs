use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde_json::json;

#[derive(Args)]
pub struct HistoryArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Limit number of entries
    #[arg(short = 'n', long)]
    pub limit: Option<usize>,

    /// Show statistics
    #[arg(long)]
    pub stats: bool,

    /// Show ASCII graph
    #[arg(long)]
    pub graph: bool,

    /// Show entries since date
    #[arg(long)]
    pub since: Option<String>,
}

/// Execute the history command
pub fn execute(
    json: bool,
    limit: Option<usize>,
    stats: bool,
    graph: bool,
    since: Option<String>,
) -> Result<()> {
    let args = HistoryArgs {
        json,
        limit,
        stats,
        graph,
        since,
    };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    // Load Layer 0 from archive to get root history
    let layer_zero_hash = crate::core::types::Hash::zero();
    if !store.archive.has_layer(&layer_zero_hash) {
        if args.json {
            println!("{}", json!({"error": "No commits found", "history": []}));
        } else {
            println!("{}", "No commits found".yellow());
        }
        return Ok(());
    }

    let content = store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&content)?;

    if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
        if args.json {
            show_history_json(&store, root_history, &args)?;
        } else {
            show_history_human(&store, root_history, &args)?;
        }
    } else if args.json {
        println!(
            "{}",
            json!({"error": "No commit history found", "history": []})
        );
    } else {
        println!("{}", "No commit history found".yellow());
    }

    Ok(())
}

fn show_history_human(
    store: &Store,
    root_history: &[serde_json::Value],
    args: &HistoryArgs,
) -> Result<()> {
    let mut entries: Vec<&serde_json::Value> = root_history.iter().collect();

    // Apply limit
    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }

    // Reverse to show newest first
    entries.reverse();

    if args.stats {
        show_history_stats(&entries)?;
        println!();
    }

    println!("{}", "Root History Analysis".green().bold());
    println!("{}", "═".repeat(50).green());

    if entries.is_empty() {
        println!("{}", "No commits found".yellow());
        return Ok(());
    }

    for (i, entry) in entries.iter().enumerate() {
        if let (Some(root_hash), Some(timestamp), Some(generation)) = (
            entry.get("root_hash").and_then(|v| v.as_str()),
            entry.get("timestamp").and_then(|v| v.as_i64()),
            entry.get("generation").and_then(|v| v.as_u64()),
        ) {
            if args.graph {
                if i == 0 {
                    println!("* {}", root_hash.cyan());
                } else {
                    println!("│");
                    println!("* {}", root_hash.cyan());
                }
                println!("│ Generation: {}", generation);
                println!("│ Date: {}", format_timestamp(timestamp));

                if let Some(layer_count) = entry.get("layer_count").and_then(|v| v.as_u64()) {
                    println!("│ Layers: {}", layer_count);
                }

                // Try to load layer for commit message
                if let Ok(layer_hash) = crate::core::types::Hash::from_hex(root_hash) {
                    if let Ok(layer) = store.load_layer(layer_hash) {
                        if let Some(message) = &layer.metadata.message {
                            println!("│ Message: {}", message.bright_white());
                        }
                        if let Some(author) = &layer.metadata.author {
                            println!("│ Author: {}", author);
                        }

                        // Show file statistics
                        let total_size: u64 = layer.files.iter().map(|f| f.size).sum();
                        println!(
                            "│ Files: {} ({} total)",
                            layer.files.len(),
                            format_bytes(total_size)
                        );
                    }
                }
            } else {
                if i > 0 {
                    println!();
                }

                println!("{} {}", "commit".yellow().bold(), root_hash.cyan());
                println!("Generation: {}", generation);
                println!("Date: {}", format_timestamp(timestamp));

                if let Some(layer_count) = entry.get("layer_count").and_then(|v| v.as_u64()) {
                    println!("Layers: {}", layer_count);
                }

                // Try to load layer for commit details
                if let Ok(layer_hash) = crate::core::types::Hash::from_hex(root_hash) {
                    if let Ok(layer) = store.load_layer(layer_hash) {
                        if let Some(message) = &layer.metadata.message {
                            println!("Message: {}", message.bright_white());
                        }
                        if let Some(author) = &layer.metadata.author {
                            println!("Author: {}", author);
                        }

                        let total_size: u64 = layer.files.iter().map(|f| f.size).sum();
                        println!(
                            "Files: {} ({} total)",
                            layer.files.len(),
                            format_bytes(total_size)
                        );
                        println!("Chunks: {}", layer.chunks.len());
                    }
                }
            }
        }
    }

    println!(
        "\n{}",
        format!(
            "Showing {} of {} commits",
            entries.len(),
            root_history.len()
        )
        .cyan()
    );
    Ok(())
}

fn show_history_json(
    store: &Store,
    root_history: &[serde_json::Value],
    args: &HistoryArgs,
) -> Result<()> {
    let mut entries: Vec<&serde_json::Value> = root_history.iter().collect();

    // Apply limit
    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }

    // Reverse to show newest first
    entries.reverse();

    let mut json_entries = Vec::new();

    for entry in entries {
        let mut json_entry = entry.clone();

        // Add layer details if available
        if let Some(root_hash_str) = entry.get("root_hash").and_then(|h| h.as_str()) {
            if let Ok(layer_hash) = crate::core::types::Hash::from_hex(root_hash_str) {
                let layer_path = store
                    .global_path()
                    .join(format!("{}.layer", layer_hash.to_hex()));
                if layer_path.exists() {
                    if let Ok(metadata) = std::fs::metadata(&layer_path) {
                        json_entry["layer_file_size"] = json!(metadata.len());
                    }

                    if let Ok(layer) = store.load_layer(layer_hash) {
                        json_entry["layer_details"] = json!({
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
        }

        json_entries.push(json_entry);
    }

    let output = if args.stats {
        let stats = calculate_history_stats(&json_entries)?;
        json!({
            "statistics": stats,
            "history": json_entries,
            "total_commits": root_history.len(),
            "showing": json_entries.len()
        })
    } else {
        json!({
            "history": json_entries,
            "total_commits": root_history.len(),
            "showing": json_entries.len()
        })
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn show_history_stats(entries: &[&serde_json::Value]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    println!("{}", "Repository Statistics".green().bold());
    println!("{}", "═".repeat(30).green());

    let total_commits = entries.len();
    println!("Total Commits: {}", total_commits);

    // Calculate repository age
    if let (Some(oldest), Some(newest)) = (entries.last(), entries.first()) {
        if let (Some(oldest_ts), Some(newest_ts)) = (
            oldest.get("timestamp").and_then(|t| t.as_i64()),
            newest.get("timestamp").and_then(|t| t.as_i64()),
        ) {
            let age_seconds = newest_ts - oldest_ts;
            let age_days = age_seconds / 86400;
            println!("Repository Age: {} days", age_days);

            if age_days > 0 {
                println!(
                    "Commit Frequency: {:.1} commits/day",
                    total_commits as f64 / age_days as f64
                );
            }
        }
    }

    println!();
    Ok(())
}

fn calculate_history_stats(entries: &[serde_json::Value]) -> Result<serde_json::Value> {
    if entries.is_empty() {
        return Ok(json!({}));
    }

    let total_commits = entries.len();

    // Calculate age and frequency
    let (age_days, commit_frequency) =
        if let (Some(oldest), Some(newest)) = (entries.last(), entries.first()) {
            if let (Some(oldest_ts), Some(newest_ts)) = (
                oldest.get("timestamp").and_then(|t| t.as_i64()),
                newest.get("timestamp").and_then(|t| t.as_i64()),
            ) {
                let age_seconds = newest_ts - oldest_ts;
                let age_days = age_seconds / 86400;
                let frequency = if age_days > 0 {
                    total_commits as f64 / age_days as f64
                } else {
                    0.0
                };
                (age_days, frequency)
            } else {
                (0, 0.0)
            }
        } else {
            (0, 0.0)
        };

    Ok(json!({
        "total_commits": total_commits,
        "repository_age_days": age_days,
        "commit_frequency_per_day": commit_frequency,
        "oldest_commit": entries.last(),
        "newest_commit": entries.first()
    }))
}

fn format_timestamp(timestamp: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    format!("{:?}", datetime)
        .split_once('.')
        .map(|(s, _)| s)
        .unwrap_or("Unknown")
        .to_string()
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
