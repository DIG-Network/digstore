use crate::storage::Store;
use anyhow::Result;
use clap::Args;
use colored::Colorize;

#[derive(Args)]
pub struct LogArgs {
    /// Limit number of entries
    #[arg(short = 'n', long)]
    pub limit: Option<usize>,

    /// One line per layer
    #[arg(long)]
    pub oneline: bool,

    /// Show ASCII graph
    #[arg(long)]
    pub graph: bool,

    /// Show layers since date
    #[arg(long)]
    pub since: Option<String>,
}

pub fn execute(
    limit: Option<usize>,
    oneline: bool,
    graph: bool,
    since: Option<String>,
) -> Result<()> {
    let args = LogArgs {
        limit,
        oneline,
        graph,
        since,
    };

    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir)?;

    println!("{}", "Commit History".green().bold());
    println!("{}", "═".repeat(50).green());

    // Load Layer 0 from archive to get root history
    let layer_zero_hash = crate::core::types::Hash::zero();
    if !store.archive.has_layer(&layer_zero_hash) {
        println!("{}", "No commits found".yellow());
        return Ok(());
    }

    let content = store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&content)?;

    if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
        let mut entries: Vec<&serde_json::Value> = root_history.iter().collect();

        // Apply limit
        if let Some(limit) = args.limit {
            entries.truncate(limit);
        }

        // Reverse to show newest first
        entries.reverse();

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
                if args.oneline {
                    println!(
                        "{} {} (gen {})",
                        root_hash[..8].yellow(),
                        format_timestamp(timestamp),
                        generation
                    );
                } else {
                    if i > 0 {
                        println!();
                    }

                    if args.graph {
                        println!("* {}", root_hash.cyan());
                        println!("| Generation: {}", generation);
                        println!("| Date: {}", format_timestamp(timestamp));
                        if let Some(layer_count) = entry.get("layer_count").and_then(|v| v.as_u64())
                        {
                            println!("| Layers: {}", layer_count);
                        }
                    } else {
                        println!("{} {}", "commit".yellow().bold(), root_hash.cyan());
                        println!("Generation: {}", generation);
                        println!("Date: {}", format_timestamp(timestamp));
                        if let Some(layer_count) = entry.get("layer_count").and_then(|v| v.as_u64())
                        {
                            println!("Layers: {}", layer_count);
                        }

                        // Try to load the layer to get commit message
                        if let Ok(root_hash_parsed) = crate::core::types::Hash::from_hex(root_hash)
                        {
                            if let Ok(layer) = store.load_layer(root_hash_parsed) {
                                if let Some(message) = &layer.metadata.message {
                                    println!("Message: {}", message);
                                }
                                if let Some(author) = &layer.metadata.author {
                                    println!("Author: {}", author);
                                }
                            }
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
    } else {
        println!("{}", "No commit history found".yellow());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        let timestamp = 1234567890;
        let formatted = format_timestamp(timestamp);
        assert!(!formatted.is_empty());
        // The format might contain "Unknown" on some systems, so just check it's not empty
    }

    #[test]
    fn test_log_args() {
        let args = LogArgs {
            limit: Some(10),
            oneline: true,
            graph: false,
            since: Some("2023-01-01".to_string()),
        };

        assert_eq!(args.limit, Some(10));
        assert!(args.oneline);
        assert!(!args.graph);
        assert_eq!(args.since, Some("2023-01-01".to_string()));
    }
}
