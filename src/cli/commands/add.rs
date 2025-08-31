//! Add command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::cli::commands::{find_repository_root};
use crate::ignore::scanner::{FilteredFileScanner, ScanPhase};
use crate::storage::parallel_processor::add_all_parallel;
use std::path::PathBuf;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Execute the add command
pub fn execute(
    paths: Vec<PathBuf>,
    recursive: bool,
    all: bool,
    force: bool,
    dry_run: bool,
    from_stdin: bool,
) -> Result<()> {
    // Find repository root
    let repo_root = find_repository_root()?
        .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .layerstore file found)"))?;

    // Open the store
    let mut store = Store::open(&repo_root)?;

    let multi_progress = MultiProgress::new();
    let main_progress = if !dry_run {
        let progress = multi_progress.add(ProgressBar::new(0));
        progress.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap()
        );
        progress.set_message("Adding files to staging...");
        Some(progress)
    } else {
        None
    };
    
    if dry_run {
        println!("{}", "Files that would be added:".bright_blue());
    } else {
        println!("{}", "Adding files to staging...".bright_blue());
    }

    let mut files_added = 0;
    let mut total_size = 0u64;
    let mut files_ignored = 0;

    if all {
        // Add all files in repository with maximum parallelism
        println!("  {} Adding all files in repository with parallel processing...", "•".cyan());
        
        if !dry_run {
            // Use high-performance parallel processing
            let stats = add_all_parallel(&repo_root, &mut store.staging, &multi_progress)?;
            
            files_added = stats.processed_files;
            total_size = stats.total_bytes;
            
            multi_progress.clear()?;
            
            // Print comprehensive performance summary
            println!();
            println!("{}", "High-Performance Add All Summary:".bold());
            println!("  • Total files processed: {}", stats.total_files);
            println!("  • Successfully staged: {}", stats.processed_files);
            println!("  • Total data processed: {:.2} MB", stats.total_bytes as f64 / 1024.0 / 1024.0);
            println!("  • Processing time: {:.2}s", stats.processing_time.as_secs_f64());
            println!("  • Processing rate: {:.1} files/s", stats.files_per_second);
            println!("  • Throughput: {:.1} MB/s", stats.bytes_per_second / 1024.0 / 1024.0);
            println!("  • Parallel efficiency: {:.1}%", stats.parallel_efficiency * 100.0);
            
            if stats.files_per_second > 1000.0 {
                println!("  {} High-performance processing achieved!", "🚀".bright_green());
            } else if stats.files_per_second > 500.0 {
                println!("  {} Good performance achieved!", "✨".bright_blue());
            }
        } else {
            // Dry run: just discover and filter files
            let mut scanner = FilteredFileScanner::new(&repo_root)?;
            let scan_result = scanner.scan_directory(&repo_root)?;
            
            files_added = scan_result.filtered_files.len();
            files_ignored = scan_result.ignored_files.len();
            
            println!();
            println!("{}", "Dry Run Summary:".bold());
            println!("  • Would process: {} files", scan_result.stats.total_filtered);
            println!("  • Would ignore: {} files ({:.1}%)", 
                     scan_result.stats.total_ignored,
                     scan_result.stats.filtering_efficiency);
        }
    } else if from_stdin {
        // Read file list from stdin
        println!("  {} Reading file list from stdin...", "•".cyan());
        use std::io::{self, BufRead};
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let file_path = PathBuf::from(line?);
            if !dry_run {
                store.add_file(&file_path)?;
            }
            println!("    {} {}", "•".green(), file_path.display());
            files_added += 1;
        }
    } else {
        // Add specified paths
        for path in &paths {
            if path.is_dir() && recursive {
                println!("  {} Adding directory: {} (recursive)", "•".cyan(), path.display());
                if !dry_run {
                    store.add_directory(path, true)?;
                }
            } else if path.is_dir() && !recursive {
                println!("  {} Skipping directory: {} (use -r for recursive)", "!".yellow(), path.display());
                continue;
            } else if path.is_file() {
                println!("  {} Adding file: {}", "•".cyan(), path.display());
                if !dry_run {
                    store.add_file(path)?;
                }
                files_added += 1;
                if let Ok(metadata) = std::fs::metadata(path) {
                    total_size += metadata.len();
                }
            } else {
                println!("  {} File not found: {}", "✗".red(), path.display());
                if !force {
                    return Err(anyhow::anyhow!("File not found: {}", path.display()));
                }
            }
        }
    }

    // Get final status
    let final_status = store.status();
    files_added = final_status.staged_files.len();
    total_size = final_status.total_staged_size;

    if let Some(progress) = main_progress {
        progress.finish_with_message("Files added to staging");
        multi_progress.clear()?;
    }

    println!();
    if dry_run {
        println!("{} {} files would be added ({} bytes)", 
            "Would add:".bright_green().bold(), 
            files_added, 
            total_size);
    } else {
        println!("{} {} files added to staging ({} bytes)", 
            "✓".green().bold(), 
            files_added, 
            total_size);
        
        if files_added > 0 {
            println!("  {} Use 'digstore commit -m \"message\"' to create a commit", "→".cyan());
        }
    }

    Ok(())
}
