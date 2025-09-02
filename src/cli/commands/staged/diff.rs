//! Stage diff command - show differences between staged files and last commit

use crate::cli::commands::find_repository_root;
use crate::storage::Store;
use anyhow::Result;
use colored::Colorize;
use serde_json::json;
use std::path::Path;

/// File change status for diff display
#[derive(Debug, Clone, serde::Serialize)]
pub enum FileChangeStatus {
    /// File is new (not in last commit)
    New,
    /// File has been modified
    Modified,
    /// File has been deleted (in commit but not staged)
    Deleted,
    /// File is unchanged (same hash)
    Unchanged,
}

/// Information about a file difference
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: FileChangeStatus,
    pub staged_hash: Option<String>,
    pub committed_hash: Option<String>,
    pub staged_size: Option<u64>,
    pub committed_size: Option<u64>,
    pub content_diff: Option<String>,
}

/// Summary statistics for stage diff
#[derive(Debug, Clone, serde::Serialize)]
pub struct StageDiffStats {
    pub total_files: usize,
    pub new_files: usize,
    pub modified_files: usize,
    pub deleted_files: usize,
    pub unchanged_files: usize,
    pub total_staged_size: u64,
    pub total_committed_size: u64,
}

/// Execute the stage diff command
pub fn execute(
    name_only: bool,
    json: bool,
    stat: bool,
    unified: usize,
    file: Option<String>,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

    // Open the store
    let mut store = Store::open(&repo_root)?;

    // Get staged files
    let staged_files = store.staging.get_all_staged_files()?;

    if staged_files.is_empty() {
        if json {
            println!(
                "{}",
                json!({
                    "diffs": [],
                    "stats": {
                        "total_files": 0,
                        "new_files": 0,
                        "modified_files": 0,
                        "deleted_files": 0,
                        "unchanged_files": 0
                    }
                })
            );
        } else {
            println!("{}", "No files staged for commit".yellow());
            println!("  → Use 'digstore add <files>' to stage files");
        }
        return Ok(());
    }

    // Calculate differences
    let diffs = calculate_stage_diffs(&store, &staged_files, file.as_deref())?;
    let stats = calculate_diff_stats(&diffs);

    if json {
        show_diff_json(&diffs, &stats)?;
    } else {
        show_diff_human(&diffs, &stats, name_only, stat, unified)?;
    }

    Ok(())
}

/// Calculate differences between staged files and last commit
fn calculate_stage_diffs(
    store: &Store,
    staged_files: &[crate::storage::binary_staging::BinaryStagedFile],
    specific_file: Option<&str>,
) -> Result<Vec<FileDiff>> {
    let mut diffs = Vec::new();

    // Filter to specific file if requested
    let files_to_process: Vec<_> = if let Some(file_filter) = specific_file {
        staged_files
            .iter()
            .filter(|f| f.path.to_string_lossy() == file_filter)
            .collect()
    } else {
        staged_files.iter().collect()
    };

    for staged_file in files_to_process {
        let diff = calculate_single_file_diff(store, staged_file)?;
        diffs.push(diff);
    }

    // Also check for deleted files (in last commit but not staged)
    if specific_file.is_none() {
        let deleted_diffs = find_deleted_files(store, staged_files)?;
        diffs.extend(deleted_diffs);
    }

    Ok(diffs)
}

/// Calculate diff for a single staged file
fn calculate_single_file_diff(
    store: &Store,
    staged_file: &crate::storage::binary_staging::BinaryStagedFile,
) -> Result<FileDiff> {
    let file_path = &staged_file.path;

    // Check if file exists in last commit
    if let Some(current_root) = store.current_root() {
        match store.get_committed_file_hash(file_path, current_root) {
            Ok(committed_hash) => {
                // File exists in last commit - check if changed
                let status = if staged_file.hash == committed_hash {
                    FileChangeStatus::Unchanged
                } else {
                    FileChangeStatus::Modified
                };

                // Get committed file size
                let committed_size = if let Ok(layer) = store.archive.get_layer(&current_root) {
                    layer
                        .files
                        .iter()
                        .find(|f| f.path == *file_path)
                        .map(|f| f.size)
                } else {
                    None
                };

                Ok(FileDiff {
                    path: file_path.display().to_string(),
                    status,
                    staged_hash: Some(staged_file.hash.to_hex()),
                    committed_hash: Some(committed_hash.to_hex()),
                    staged_size: Some(staged_file.size),
                    committed_size,
                    content_diff: None, // Will be populated later if needed
                })
            },
            Err(_) => {
                // File not in last commit - it's new
                Ok(FileDiff {
                    path: file_path.display().to_string(),
                    status: FileChangeStatus::New,
                    staged_hash: Some(staged_file.hash.to_hex()),
                    committed_hash: None,
                    staged_size: Some(staged_file.size),
                    committed_size: None,
                    content_diff: None,
                })
            },
        }
    } else {
        // No previous commit - all files are new
        Ok(FileDiff {
            path: file_path.display().to_string(),
            status: FileChangeStatus::New,
            staged_hash: Some(staged_file.hash.to_hex()),
            committed_hash: None,
            staged_size: Some(staged_file.size),
            committed_size: None,
            content_diff: None,
        })
    }
}

/// Find files that were deleted (in last commit but not staged)
fn find_deleted_files(
    store: &Store,
    staged_files: &[crate::storage::binary_staging::BinaryStagedFile],
) -> Result<Vec<FileDiff>> {
    let mut deleted_diffs = Vec::new();

    if let Some(current_root) = store.current_root() {
        if let Ok(layer) = store.archive.get_layer(&current_root) {
            // Create set of staged file paths for quick lookup
            let staged_paths: std::collections::HashSet<_> =
                staged_files.iter().map(|f| f.path.clone()).collect();

            // Check each committed file
            for file_entry in &layer.files {
                if !staged_paths.contains(&file_entry.path) {
                    // File is in commit but not staged - it's deleted
                    deleted_diffs.push(FileDiff {
                        path: file_entry.path.display().to_string(),
                        status: FileChangeStatus::Deleted,
                        staged_hash: None,
                        committed_hash: Some(file_entry.hash.to_hex()),
                        staged_size: None,
                        committed_size: Some(file_entry.size),
                        content_diff: None,
                    });
                }
            }
        }
    }

    Ok(deleted_diffs)
}

/// Calculate summary statistics
fn calculate_diff_stats(diffs: &[FileDiff]) -> StageDiffStats {
    let mut stats = StageDiffStats {
        total_files: diffs.len(),
        new_files: 0,
        modified_files: 0,
        deleted_files: 0,
        unchanged_files: 0,
        total_staged_size: 0,
        total_committed_size: 0,
    };

    for diff in diffs {
        match diff.status {
            FileChangeStatus::New => stats.new_files += 1,
            FileChangeStatus::Modified => stats.modified_files += 1,
            FileChangeStatus::Deleted => stats.deleted_files += 1,
            FileChangeStatus::Unchanged => stats.unchanged_files += 1,
        }

        if let Some(size) = diff.staged_size {
            stats.total_staged_size += size;
        }
        if let Some(size) = diff.committed_size {
            stats.total_committed_size += size;
        }
    }

    stats
}

/// Show diff in JSON format
fn show_diff_json(diffs: &[FileDiff], stats: &StageDiffStats) -> Result<()> {
    let output = json!({
        "diffs": diffs,
        "stats": stats
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Show diff in human-readable format
fn show_diff_human(
    diffs: &[FileDiff],
    stats: &StageDiffStats,
    name_only: bool,
    show_stat: bool,
    _unified: usize,
) -> Result<()> {
    println!();
    println!("{}", "Stage Diff".bold());
    println!("{}", "═══════════".bold());
    println!();

    if show_stat {
        println!("{}", "Summary Statistics:".bold());
        println!(
            "  New files: {} (+{})",
            stats.new_files.to_string().green(),
            format_size(stats.total_staged_size)
        );
        println!(
            "  Modified files: {} (Δ{})",
            stats.modified_files.to_string().yellow(),
            format_size_diff(stats.total_staged_size, stats.total_committed_size)
        );
        println!(
            "  Deleted files: {} (-{})",
            stats.deleted_files.to_string().red(),
            format_size(stats.total_committed_size)
        );
        if stats.unchanged_files > 0 {
            println!(
                "  Unchanged files: {} (skipped)",
                stats.unchanged_files.to_string().dimmed()
            );
        }
        println!();
    }

    if diffs.is_empty() {
        println!("{}", "No differences found".yellow());
        return Ok(());
    }

    // Group by status for better display
    let new_files: Vec<_> = diffs
        .iter()
        .filter(|d| matches!(d.status, FileChangeStatus::New))
        .collect();
    let modified_files: Vec<_> = diffs
        .iter()
        .filter(|d| matches!(d.status, FileChangeStatus::Modified))
        .collect();
    let deleted_files: Vec<_> = diffs
        .iter()
        .filter(|d| matches!(d.status, FileChangeStatus::Deleted))
        .collect();

    // Show new files
    if !new_files.is_empty() {
        println!("{}", "New files:".green().bold());
        for diff in new_files {
            if name_only {
                println!("  {} {}", "+".green(), diff.path.green());
            } else {
                println!(
                    "  {} {} ({})",
                    "+".green(),
                    diff.path.green(),
                    format_size(diff.staged_size.unwrap_or(0))
                );
            }
        }
        println!();
    }

    // Show modified files
    if !modified_files.is_empty() {
        println!("{}", "Modified files:".yellow().bold());
        for diff in modified_files {
            if name_only {
                println!("  {} {}", "M".yellow(), diff.path.yellow());
            } else {
                let size_change = format_size_diff(
                    diff.staged_size.unwrap_or(0),
                    diff.committed_size.unwrap_or(0),
                );
                println!(
                    "  {} {} ({})",
                    "M".yellow(),
                    diff.path.yellow(),
                    size_change
                );
            }
        }
        println!();
    }

    // Show deleted files
    if !deleted_files.is_empty() {
        println!("{}", "Deleted files:".red().bold());
        for diff in deleted_files {
            if name_only {
                println!("  {} {}", "-".red(), diff.path.red());
            } else {
                println!(
                    "  {} {} (-{})",
                    "-".red(),
                    diff.path.red(),
                    format_size(diff.committed_size.unwrap_or(0))
                );
            }
        }
        println!();
    }

    // Show summary
    println!("{}", format!("Total changes: {} files", diffs.len()).cyan());
    if !name_only {
        println!("  → Use 'digstore commit -m \"message\"' to commit these changes");
        println!("  → Use 'digstore staged' to see detailed file list");
    }

    Ok(())
}

/// Format file size in human-readable format
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
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

/// Format size difference (staged vs committed)
fn format_size_diff(staged_size: u64, committed_size: u64) -> String {
    if staged_size > committed_size {
        let diff = staged_size - committed_size;
        format!("+{}", format_size(diff)).green().to_string()
    } else if staged_size < committed_size {
        let diff = committed_size - staged_size;
        format!("-{}", format_size(diff)).red().to_string()
    } else {
        "no change".dimmed().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
    }

    #[test]
    fn test_format_size_diff() {
        // Note: These tests check the logic, but the colored output will contain ANSI codes
        let result = format_size_diff(2048, 1024);
        assert!(result.contains("1.0 KB")); // Should show +1.0 KB

        let result = format_size_diff(1024, 2048);
        assert!(result.contains("1.0 KB")); // Should show -1.0 KB

        let result = format_size_diff(1024, 1024);
        assert!(result.contains("no change")); // Should show no change
    }

    #[test]
    fn test_diff_stats_calculation() {
        let diffs = vec![
            FileDiff {
                path: "new.txt".to_string(),
                status: FileChangeStatus::New,
                staged_hash: Some("abc123".to_string()),
                committed_hash: None,
                staged_size: Some(100),
                committed_size: None,
                content_diff: None,
            },
            FileDiff {
                path: "modified.txt".to_string(),
                status: FileChangeStatus::Modified,
                staged_hash: Some("def456".to_string()),
                committed_hash: Some("ghi789".to_string()),
                staged_size: Some(200),
                committed_size: Some(150),
                content_diff: None,
            },
        ];

        let stats = calculate_diff_stats(&diffs);

        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.new_files, 1);
        assert_eq!(stats.modified_files, 1);
        assert_eq!(stats.deleted_files, 0);
        assert_eq!(stats.total_staged_size, 300);
        assert_eq!(stats.total_committed_size, 150);
    }
}
