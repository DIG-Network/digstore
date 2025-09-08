//! File scanner with .digignore filtering and progress reporting

use crate::ignore::checker::{IgnoreChecker, IgnoreResult};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

/// Phase of file scanning operation
#[derive(Debug, Clone, PartialEq)]
pub enum ScanPhase {
    /// Discovering files in directories
    Discovery,
    /// Applying .digignore filters
    Filtering,
    /// Processing filtered files
    Processing,
    /// Scan completed successfully
    Complete,
}

/// Progress information during scanning
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Current phase of scanning
    pub phase: ScanPhase,
    /// Total files discovered so far
    pub files_discovered: usize,
    /// Files remaining after filtering
    pub files_filtered: usize,
    /// Files processed so far
    pub files_processed: usize,
    /// Current file being processed (if any)
    pub current_file: Option<PathBuf>,
    /// Elapsed time since scan started
    pub elapsed: Duration,
    /// Files ignored by patterns
    pub files_ignored: usize,
    /// Processing rate (files per second)
    pub processing_rate: f64,
}

/// Result of file scanning operation
#[derive(Debug)]
pub struct ScanResult {
    /// Files that passed filtering
    pub filtered_files: Vec<PathBuf>,
    /// Files that were ignored and their reasons
    pub ignored_files: Vec<(PathBuf, String)>,
    /// Total scanning time
    pub total_time: Duration,
    /// Final statistics
    pub stats: ScanStats,
}

/// Statistics from scanning operation
#[derive(Debug, Clone)]
pub struct ScanStats {
    /// Total files discovered
    pub total_discovered: usize,
    /// Files that passed filtering
    pub total_filtered: usize,
    /// Files ignored by patterns
    pub total_ignored: usize,
    /// Filtering efficiency (percentage ignored)
    pub filtering_efficiency: f64,
    /// Discovery rate (files per second)
    pub discovery_rate: f64,
    /// Processing rate (files per second)
    pub processing_rate: f64,
}

/// File scanner with .digignore filtering and progress reporting
pub struct FilteredFileScanner {
    /// Ignore checker for filtering
    ignore_checker: IgnoreChecker,
    /// Progress callback function
    progress_callback: Option<Box<dyn Fn(&ScanProgress) + Send + Sync>>,
    /// Whether to follow symbolic links
    follow_links: bool,
    /// Maximum depth for directory traversal (None = unlimited)
    max_depth: Option<usize>,
}

impl FilteredFileScanner {
    /// Create a new filtered file scanner
    pub fn new(repo_root: &Path) -> Result<Self> {
        let ignore_checker = IgnoreChecker::new(repo_root)?;

        Ok(Self {
            ignore_checker,
            progress_callback: None,
            follow_links: false,
            max_depth: None,
        })
    }

    /// Set progress callback for real-time updates
    pub fn with_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(&ScanProgress) + Send + Sync + 'static,
    {
        self.progress_callback = Some(Box::new(callback));
        self
    }

    /// Set whether to follow symbolic links
    pub fn follow_links(mut self, follow: bool) -> Self {
        self.follow_links = follow;
        self
    }

    /// Set maximum depth for directory traversal
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Scan a single directory with filtering
    pub fn scan_directory(&mut self, dir_path: &Path) -> Result<ScanResult> {
        let start_time = Instant::now();
        let mut progress = ScanProgress {
            phase: ScanPhase::Discovery,
            files_discovered: 0,
            files_filtered: 0,
            files_processed: 0,
            current_file: None,
            elapsed: Duration::default(),
            files_ignored: 0,
            processing_rate: 0.0,
        };

        // Phase 1: Discovery
        self.report_progress(&progress);

        let discovered_files = self.discover_files(dir_path, &mut progress)?;

        // Phase 2: Filtering
        progress.phase = ScanPhase::Filtering;
        progress.files_discovered = discovered_files.len();
        self.report_progress(&progress);

        let (filtered_files, ignored_files) = self.filter_files(discovered_files, &mut progress)?;

        // Phase 3: Complete
        progress.phase = ScanPhase::Complete;
        progress.files_filtered = filtered_files.len();
        progress.files_ignored = ignored_files.len();
        progress.elapsed = start_time.elapsed();
        self.report_progress(&progress);

        let stats = ScanStats {
            total_discovered: progress.files_discovered,
            total_filtered: progress.files_filtered,
            total_ignored: progress.files_ignored,
            filtering_efficiency: if progress.files_discovered > 0 {
                (progress.files_ignored as f64 / progress.files_discovered as f64) * 100.0
            } else {
                0.0
            },
            discovery_rate: if progress.elapsed.as_secs_f64() > 0.0 {
                progress.files_discovered as f64 / progress.elapsed.as_secs_f64()
            } else {
                0.0
            },
            processing_rate: if progress.elapsed.as_secs_f64() > 0.0 {
                progress.files_filtered as f64 / progress.elapsed.as_secs_f64()
            } else {
                0.0
            },
        };

        Ok(ScanResult {
            filtered_files,
            ignored_files,
            total_time: progress.elapsed,
            stats,
        })
    }

    /// Scan multiple paths (for digstore add -A)
    pub fn scan_all(&mut self, paths: &[PathBuf]) -> Result<ScanResult> {
        let start_time = Instant::now();
        let mut all_discovered = Vec::new();
        let mut progress = ScanProgress {
            phase: ScanPhase::Discovery,
            files_discovered: 0,
            files_filtered: 0,
            files_processed: 0,
            current_file: None,
            elapsed: Duration::default(),
            files_ignored: 0,
            processing_rate: 0.0,
        };

        // Discover all files from all paths
        for path in paths {
            if path.is_dir() {
                let mut discovered = self.discover_files(path, &mut progress)?;
                all_discovered.append(&mut discovered);
            } else if path.exists() {
                all_discovered.push(path.clone());
                progress.files_discovered += 1;
                self.report_progress(&progress);
            }
        }

        // Filter discovered files
        progress.phase = ScanPhase::Filtering;
        self.report_progress(&progress);

        let (filtered_files, ignored_files) = self.filter_files(all_discovered, &mut progress)?;

        progress.phase = ScanPhase::Complete;
        progress.files_filtered = filtered_files.len();
        progress.files_ignored = ignored_files.len();
        progress.elapsed = start_time.elapsed();
        self.report_progress(&progress);

        let stats = ScanStats {
            total_discovered: progress.files_discovered,
            total_filtered: progress.files_filtered,
            total_ignored: progress.files_ignored,
            filtering_efficiency: if progress.files_discovered > 0 {
                (progress.files_ignored as f64 / progress.files_discovered as f64) * 100.0
            } else {
                0.0
            },
            discovery_rate: if progress.elapsed.as_secs_f64() > 0.0 {
                progress.files_discovered as f64 / progress.elapsed.as_secs_f64()
            } else {
                0.0
            },
            processing_rate: if progress.elapsed.as_secs_f64() > 0.0 {
                progress.files_filtered as f64 / progress.elapsed.as_secs_f64()
            } else {
                0.0
            },
        };

        Ok(ScanResult {
            filtered_files,
            ignored_files,
            total_time: progress.elapsed,
            stats,
        })
    }

    /// Discover all files in a directory
    fn discover_files(
        &mut self,
        dir_path: &Path,
        progress: &mut ScanProgress,
    ) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut walker = WalkDir::new(dir_path).follow_links(self.follow_links);

        if let Some(depth) = self.max_depth {
            walker = walker.max_depth(depth);
        }

        for entry in walker.into_iter() {
            match entry {
                Ok(entry) => {
                    if entry.file_type().is_file() {
                        files.push(entry.path().to_path_buf());
                        progress.files_discovered += 1;
                        progress.current_file = Some(entry.path().to_path_buf());

                        // Report progress every 100 files during discovery
                        if progress.files_discovered % 100 == 0 {
                            self.report_progress(progress);
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Warning: Error accessing file: {}", e);
                },
            }
        }

        Ok(files)
    }

    /// Filter files through .digignore rules
    fn filter_files(
        &mut self,
        files: Vec<PathBuf>,
        progress: &mut ScanProgress,
    ) -> Result<(Vec<PathBuf>, Vec<(PathBuf, String)>)> {
        let mut filtered_files = Vec::new();
        let mut ignored_files = Vec::new();

        for (index, file_path) in files.into_iter().enumerate() {
            progress.current_file = Some(file_path.clone());
            progress.files_processed = index + 1;

            match self.ignore_checker.is_ignored(&file_path) {
                IgnoreResult::Included => {
                    filtered_files.push(file_path);
                },
                IgnoreResult::Ignored(reason) => {
                    ignored_files.push((file_path, reason));
                    progress.files_ignored += 1;
                },
                IgnoreResult::IncludedByNegation(_reason) => {
                    filtered_files.push(file_path);
                },
            }

            // Report progress every 50 files during filtering
            if index % 50 == 0 {
                self.report_progress(progress);
            }
        }

        Ok((filtered_files, ignored_files))
    }

    /// Report progress to callback
    fn report_progress(&self, progress: &ScanProgress) {
        if let Some(callback) = &self.progress_callback {
            callback(progress);
        }
    }

    /// Reload ignore rules
    pub fn reload_ignore_rules(&mut self) -> Result<()> {
        self.ignore_checker.reload()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[test]
    fn test_scan_directory_with_ignore() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create .digignore
        fs::write(root.join(".digignore"), "*.tmp\n*.log\n")?;

        // Create test files
        fs::write(root.join("keep.txt"), "content")?;
        fs::write(root.join("ignore.tmp"), "content")?;
        fs::write(root.join("ignore.log"), "content")?;

        let progress_calls = Arc::new(Mutex::new(Vec::new()));
        let progress_calls_clone = Arc::clone(&progress_calls);

        let mut scanner = FilteredFileScanner::new(root)?.with_progress(move |progress| {
            progress_calls_clone
                .lock()
                .unwrap()
                .push(progress.phase.clone());
        });

        let result = scanner.scan_directory(root)?;

        // Should have found keep.txt but ignored the others
        println!("Filtered files: {:?}", result.filtered_files);
        println!("Ignored files: {:?}", result.ignored_files);
        println!("Stats: {:?}", result.stats);

        // Filter out .digignore file from results for test
        let non_digignore_filtered: Vec<_> = result
            .filtered_files
            .iter()
            .filter(|p| !p.file_name().map(|n| n == ".digignore").unwrap_or(false))
            .collect();
        let non_digignore_ignored: Vec<_> = result
            .ignored_files
            .iter()
            .filter(|(p, _)| !p.file_name().map(|n| n == ".digignore").unwrap_or(false))
            .collect();

        assert_eq!(non_digignore_filtered.len(), 1); // keep.txt
        assert_eq!(non_digignore_ignored.len(), 2); // ignore.tmp and ignore.log

        // Should have called progress callback
        let calls = progress_calls.lock().unwrap();
        assert!(calls.contains(&ScanPhase::Discovery));
        assert!(calls.contains(&ScanPhase::Filtering));
        assert!(calls.contains(&ScanPhase::Complete));

        Ok(())
    }

    #[test]
    fn test_scan_all_multiple_paths() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create directories
        fs::create_dir(root.join("dir1"))?;
        fs::create_dir(root.join("dir2"))?;

        // Create .digignore
        fs::write(root.join(".digignore"), "*.tmp\n")?;

        // Create files
        fs::write(root.join("dir1/file1.txt"), "content")?;
        fs::write(root.join("dir1/file1.tmp"), "content")?;
        fs::write(root.join("dir2/file2.txt"), "content")?;
        fs::write(root.join("dir2/file2.tmp"), "content")?;

        let mut scanner = FilteredFileScanner::new(root)?;

        let paths = vec![root.join("dir1"), root.join("dir2")];

        let result = scanner.scan_all(&paths)?;

        // Should find 2 .txt files, ignore 2 .tmp files
        assert_eq!(result.filtered_files.len(), 2);
        assert_eq!(result.ignored_files.len(), 2);
        assert_eq!(result.stats.filtering_efficiency, 50.0);

        Ok(())
    }
}
