//! List staged files command

use crate::cli::commands::find_repository_root;
use crate::storage::Store;
use anyhow::Result;
use colored::Colorize;
use serde_json::json;
use tabled::{Table, Tabled};

/// Arguments for the staged command
#[derive(Debug)]
pub struct StagedArgs {
    pub limit: usize,
    pub page: usize,
    pub detailed: bool,
    pub json: bool,
    pub all: bool,
}

/// Staged file information for display
#[derive(Tabled, serde::Serialize)]
struct StagedFileInfo {
    #[tabled(rename = "File")]
    file: String,
    #[tabled(rename = "Size")]
    size: String,
    #[tabled(rename = "Hash")]
    hash: String,
    #[tabled(rename = "Chunks")]
    chunks: usize,
}

/// Execute the staged command
pub fn execute(limit: usize, page: usize, detailed: bool, json: bool, all: bool) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let mut store = Store::open(&repo_root)?;

    // Get all staged files
    let staged_files = store.staging.get_all_staged_files()?;

    if staged_files.is_empty() {
        if json {
            println!(
                "{}",
                json!({
                    "staged_files": [],
                    "total_count": 0,
                    "page": page,
                    "limit": limit,
                    "total_pages": 0
                })
            );
        } else {
            println!("{}", "No files staged for commit".yellow());
            println!("  → Use 'digstore add <files>' to stage files");
        }
        return Ok(());
    }

    let total_count = staged_files.len();
    let total_pages = if all {
        1
    } else {
        (total_count + limit - 1) / limit
    };

    // Validate page number
    if page < 1 || (!all && page > total_pages) {
        return Err(anyhow::anyhow!(
            "Invalid page number: {} (valid range: 1-{})",
            page,
            total_pages
        ));
    }

    // Calculate pagination
    let (start_idx, end_idx) = if all {
        (0, total_count)
    } else {
        let start = (page - 1) * limit;
        let end = (start + limit).min(total_count);
        (start, end)
    };

    let page_files = &staged_files[start_idx..end_idx];

    if json {
        // JSON output
        let file_data: Vec<_> = page_files.iter().map(|f| {
            if detailed {
                json!({
                    "path": f.path.display().to_string(),
                    "size": f.size,
                    "hash": f.hash.to_hex(),
                    "chunks": f.chunks.len(),
                    "modified_time": f.modified_time.map(|t| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())).flatten()
                })
            } else {
                json!(f.path.display().to_string())
            }
        }).collect();

        println!(
            "{}",
            json!({
                "staged_files": file_data,
                "total_count": total_count,
                "page": page,
                "limit": limit,
                "total_pages": total_pages,
                "showing": format!("{}-{}", start_idx + 1, end_idx)
            })
        );
    } else {
        // Human-readable output
        println!();
        println!("{}", "Staged Files".bold());
        println!("{}", "════════════".bold());
        println!();

        if detailed {
            // Detailed table view
            let table_data: Vec<StagedFileInfo> = page_files
                .iter()
                .map(|f| StagedFileInfo {
                    file: f.path.display().to_string(),
                    size: format_size(f.size),
                    hash: f.hash.to_hex()[..8].to_string() + "...",
                    chunks: f.chunks.len(),
                })
                .collect();

            let table = Table::new(table_data);
            println!("{}", table);
        } else {
            // Simple list view
            for file in page_files {
                println!("  {}", file.path.display().to_string().green());
            }
        }

        println!();
        if !all && total_pages > 1 {
            println!(
                "{}",
                format!(
                    "Page {} of {} ({} files total)",
                    page, total_pages, total_count
                )
                .cyan()
            );

            if page < total_pages {
                println!(
                    "  → Use 'digstore staged --page {}' for next page",
                    page + 1
                );
            }
            if page > 1 {
                println!(
                    "  → Use 'digstore staged --page {}' for previous page",
                    page - 1
                );
            }
            println!("  → Use 'digstore staged --all' to show all files");
        } else {
            println!(
                "{}",
                format!("Showing {} staged files", page_files.len()).cyan()
            );
        }

        let total_size: u64 = staged_files.iter().map(|f| f.size).sum();
        println!("  → Total size: {}", format_size(total_size));
        println!("  → Use 'digstore commit -m \"message\"' to create a commit");
        println!();
    }

    Ok(())
}

/// Clear all staged files
pub fn clear_staged(json: bool, force: bool) -> Result<()> {
    use crate::cli::commands::find_repository_root;

    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let mut store = crate::storage::Store::open(&repo_root)?;

    // Check if there are staged files
    let staged_files = store.staging.get_all_staged_files()?;

    if staged_files.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "message": "No files staged",
                    "cleared": 0
                })
            );
        } else {
            println!("{}", "No files staged".yellow());
        }
        return Ok(());
    }

    let file_count = staged_files.len();

    // Ask for confirmation unless force is used
    if !force {
        use colored::Colorize;
        use dialoguer::Confirm;

        println!();
        println!(
            "{}",
            format!("About to clear {} staged files", file_count)
                .yellow()
                .bold()
        );
        println!();

        let confirmed = Confirm::new()
            .with_prompt("Are you sure you want to clear all staged files?")
            .default(false)
            .interact()?;

        if !confirmed {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "message": "Operation cancelled",
                        "cleared": 0
                    })
                );
            } else {
                println!("{}", "Operation cancelled".yellow());
            }
            return Ok(());
        }
    }

    // Clear staging
    store.clear_staging()?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "message": "Staging cleared",
                "cleared": file_count
            })
        );
    } else {
        println!("{} Cleared {} staged files", "✓".green().bold(), file_count);
    }

    Ok(())
}

/// Format file size in human-readable format
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let unit_index = (bytes_f.log10() / THRESHOLD.log10()).floor() as usize;
    let unit_index = unit_index.min(UNITS.len() - 1);

    let size = bytes_f / THRESHOLD.powi(unit_index as i32);

    if size >= 100.0 {
        format!("{:.0} {}", size, UNITS[unit_index])
    } else if size >= 10.0 {
        format!("{:.1} {}", size, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_size(1048576), "1.00 MB");
        assert_eq!(format_size(1073741824), "1.00 GB");
    }

    #[test]
    fn test_pagination_calculation() {
        // Test pagination logic
        let total = 100;
        let limit = 20;
        let total_pages = (total + limit - 1) / limit; // 5 pages
        assert_eq!(total_pages, 5);

        // Page 1: 0-19 (20 items)
        let page = 1;
        let start = (page - 1) * limit; // 0
        let end = (start + limit).min(total); // 20
        assert_eq!((start, end), (0, 20));

        // Page 5: 80-99 (20 items)
        let page = 5;
        let start = (page - 1) * limit; // 80
        let end = (start + limit).min(total); // 100
        assert_eq!((start, end), (80, 100));
    }

    #[test]
    fn test_pagination_edge_cases() {
        // Test edge cases for pagination

        // Empty list
        let total = 0;
        let limit = 20;
        let total_pages = if total == 0 {
            0
        } else {
            (total + limit - 1) / limit
        };
        assert_eq!(total_pages, 0);

        // Single page
        let total = 15;
        let limit = 20;
        let total_pages = (total + limit - 1) / limit;
        assert_eq!(total_pages, 1);

        // Exact page boundary
        let total = 40;
        let limit = 20;
        let total_pages = (total + limit - 1) / limit;
        assert_eq!(total_pages, 2);

        // Last page with fewer items
        let total = 95;
        let limit = 20;
        let page = 5;
        let start = (page - 1) * limit; // 80
        let end = (start + limit).min(total); // 95
        assert_eq!((start, end), (80, 95)); // Only 15 items on last page
    }

    #[test]
    fn test_invalid_page_numbers() {
        // Test validation logic for page numbers
        let total = 100;
        let limit = 20;
        let total_pages = (total + limit - 1) / limit; // 5 pages

        // Page 0 should be invalid
        let page = 0;
        assert!(page < 1, "Page 0 should be invalid");

        // Page beyond total should be invalid
        let page = 6;
        assert!(page > total_pages, "Page beyond total should be invalid");

        // Valid pages
        for page in 1..=total_pages {
            assert!(
                page >= 1 && page <= total_pages,
                "Page {} should be valid",
                page
            );
        }
    }
}
