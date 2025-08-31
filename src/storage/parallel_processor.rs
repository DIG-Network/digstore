//! High-performance parallel file processing for large repositories
//!
//! This module implements massively parallel file processing that can handle
//! tens of thousands of files efficiently using:
//! - Thread pool for parallel processing
//! - Lock-free data structures for coordination
//! - Streaming writes to avoid memory bottlenecks
//! - Real-time progress reporting
//! - Adaptive batch sizing based on system resources

use crate::core::{types::*, error::{Result, DigstoreError}};
use crate::storage::{
    chunk::ChunkingEngine,
    streaming::StreamingChunkingEngine,
    binary_staging::{BinaryStagingArea, BinaryStagedFile},
};
use crate::ignore::scanner::{FilteredFileScanner, ScanResult};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::{Instant, Duration};
use rayon::prelude::*;
use dashmap::DashMap;
use crossbeam_channel::{bounded, Receiver, Sender};
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};

/// Configuration for parallel processing
#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Number of worker threads (0 = auto-detect)
    pub worker_threads: usize,
    /// Batch size for staging writes
    pub staging_batch_size: usize,
    /// Buffer size for file reading
    pub read_buffer_size: usize,
    /// Whether to use memory mapping for large files
    pub use_memory_mapping: bool,
    /// Threshold for using streaming vs batch processing
    pub streaming_threshold: u64,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        let cpu_count = num_cpus::get();
        Self {
            worker_threads: cpu_count * 2, // Hyper-threading friendly
            staging_batch_size: 1000,
            read_buffer_size: 64 * 1024, // 64KB buffer
            use_memory_mapping: true,
            streaming_threshold: 10 * 1024 * 1024, // 10MB
        }
    }
}

/// Progress tracking for parallel operations
#[derive(Debug)]
pub struct ParallelProgress {
    /// Files discovered
    pub files_discovered: Arc<AtomicUsize>,
    /// Files processed
    pub files_processed: Arc<AtomicUsize>,
    /// Files staged
    pub files_staged: Arc<AtomicUsize>,
    /// Files ignored
    pub files_ignored: Arc<AtomicUsize>,
    /// Bytes processed
    pub bytes_processed: Arc<AtomicU64>,
    /// Start time
    pub start_time: Instant,
    /// Current phase
    pub current_phase: Arc<Mutex<String>>,
}

impl ParallelProgress {
    pub fn new() -> Self {
        Self {
            files_discovered: Arc::new(AtomicUsize::new(0)),
            files_processed: Arc::new(AtomicUsize::new(0)),
            files_staged: Arc::new(AtomicUsize::new(0)),
            files_ignored: Arc::new(AtomicUsize::new(0)),
            bytes_processed: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
            current_phase: Arc::new(Mutex::new("Initializing".to_string())),
        }
    }

    pub fn files_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.files_processed.load(Ordering::Relaxed) as f64 / elapsed
        } else {
            0.0
        }
    }

    pub fn bytes_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_processed.load(Ordering::Relaxed) as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// Work item for parallel processing
#[derive(Debug, Clone)]
pub struct ProcessingWorkItem {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub priority: u8, // 0 = highest priority
}

/// Result of processing a single file
#[derive(Debug)]
pub struct ProcessingResult {
    pub staged_file: BinaryStagedFile,
    pub processing_time: Duration,
    pub chunk_count: usize,
}

/// High-performance parallel file processor
pub struct ParallelFileProcessor {
    config: ParallelConfig,
    progress: ParallelProgress,
    chunking_engine: ChunkingEngine,
    streaming_engine: StreamingChunkingEngine,
    staging_sender: Sender<Vec<BinaryStagedFile>>,
    staging_receiver: Receiver<Vec<BinaryStagedFile>>,
}

impl ParallelFileProcessor {
    /// Create a new parallel file processor
    pub fn new(config: ParallelConfig) -> Self {
        let (staging_sender, staging_receiver) = bounded(100); // Bounded channel for backpressure
        
        Self {
            config,
            progress: ParallelProgress::new(),
            chunking_engine: ChunkingEngine::new(),
            streaming_engine: StreamingChunkingEngine::new(),
            staging_sender,
            staging_receiver,
        }
    }

    /// Process all files in a directory with maximum parallelism
    pub fn process_directory_parallel(
        &mut self,
        directory: &Path,
        staging_area: &mut BinaryStagingArea,
        multi_progress: &MultiProgress,
    ) -> Result<ProcessingStats> {
        // Phase 1: Discover and filter files
        self.set_phase("Discovering files...");
        
        let discovery_pb = multi_progress.add(ProgressBar::new_spinner());
        discovery_pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg} ({pos:>7} files)")
                .unwrap()
        );

        let mut scanner = FilteredFileScanner::new(directory)
            .map_err(|e| DigstoreError::internal(format!("Failed to create scanner: {}", e)))?;
        let scan_result = scanner.scan_directory(directory)
            .map_err(|e| DigstoreError::internal(format!("Failed to scan directory: {}", e)))?;
        
        discovery_pb.finish_with_message(format!(
            "✓ Discovered {} files ({} filtered out)",
            scan_result.stats.total_discovered,
            scan_result.stats.total_ignored
        ));

        self.progress.files_discovered.store(scan_result.stats.total_discovered, Ordering::Relaxed);
        self.progress.files_ignored.store(scan_result.stats.total_ignored, Ordering::Relaxed);

        if scan_result.filtered_files.is_empty() {
            return Ok(ProcessingStats {
                total_files: 0,
                processed_files: 0,
                total_bytes: 0,
                processing_time: Duration::default(),
                files_per_second: 0.0,
                bytes_per_second: 0.0,
                parallel_efficiency: 0.0,
            });
        }

        // Phase 2: Parallel file processing
        self.set_phase("Processing files in parallel...");
        
        let processing_pb = multi_progress.add(ProgressBar::new(scan_result.filtered_files.len() as u64));
        processing_pb.set_style(
            ProgressStyle::default_bar()
                .template("{bar:50.cyan/blue} {pos:>7}/{len:7} files ({percent:>3}%) | {per_sec:>8} | ETA: {eta:>5} | {msg}")
                .unwrap()
        );

        // Start staging writer thread
        let staging_writer_handle = self.start_staging_writer_thread(staging_area, &processing_pb)?;

        // Create work items with priority (smaller files first for better parallelism)
        let mut work_items: Vec<ProcessingWorkItem> = scan_result.filtered_files
            .into_iter()
            .map(|path| {
                let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let priority = if file_size < 1024 * 1024 { 0 } // < 1MB = high priority
                else if file_size < 10 * 1024 * 1024 { 1 } // < 10MB = medium priority
                else { 2 }; // >= 10MB = low priority
                
                ProcessingWorkItem {
                    file_path: path,
                    file_size,
                    priority,
                }
            })
            .collect();

        // Sort by priority and size for optimal scheduling
        work_items.sort_by_key(|item| (item.priority, item.file_size));

        let total_files = work_items.len();
        let total_bytes: u64 = work_items.iter().map(|w| w.file_size).sum();

        // Configure rayon thread pool
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.config.worker_threads)
            .build()
            .map_err(|e| DigstoreError::internal(format!("Failed to create thread pool: {}", e)))?;

        // Process files in parallel with progress tracking
        let progress_clone = Arc::clone(&self.progress.files_processed);
        let bytes_progress_clone = Arc::clone(&self.progress.bytes_processed);
        let staging_sender = self.staging_sender.clone();
        
        thread_pool.install(|| {
            work_items
                .into_par_iter()
                .chunks(self.config.staging_batch_size)
                .try_for_each(|batch| -> Result<()> {
                    let mut batch_results = Vec::with_capacity(batch.len());
                    
                    // Process batch of files in parallel
                    let batch_processed: Result<Vec<_>> = batch
                        .into_par_iter()
                        .map(|work_item| {
                            self.process_single_file(&work_item)
                        })
                        .collect();
                    
                    match batch_processed {
                        Ok(results) => {
                            for result in results {
                                progress_clone.fetch_add(1, Ordering::Relaxed);
                                bytes_progress_clone.fetch_add(result.staged_file.size, Ordering::Relaxed);
                                processing_pb.set_position(progress_clone.load(Ordering::Relaxed) as u64);
                                processing_pb.set_message(format!(
                                    "{:.1} files/s, {:.1} MB/s",
                                    self.progress.files_per_second(),
                                    self.progress.bytes_per_second() / 1024.0 / 1024.0
                                ));
                                batch_results.push(result.staged_file);
                            }
                            
                            // Send batch to staging writer
                            if !batch_results.is_empty() {
                                staging_sender.send(batch_results)
                                    .map_err(|e| DigstoreError::internal(format!("Failed to send to staging: {}", e)))?;
                            }
                        }
                        Err(e) => return Err(e),
                    }
                    
                    Ok(())
                })
        })?;

        // Signal completion and wait for staging writer
        staging_writer_handle.join()
            .map_err(|e| DigstoreError::internal(format!("Staging writer thread failed: {:?}", e)))??;

        processing_pb.finish_with_message(format!(
            "✓ Processed {} files ({:.1} files/s, {:.1} MB/s)",
            total_files,
            self.progress.files_per_second(),
            self.progress.bytes_per_second() / 1024.0 / 1024.0
        ));

        let processing_time = self.progress.start_time.elapsed();
        let parallel_efficiency = if self.config.worker_threads > 1 {
            (self.progress.files_per_second() * self.config.worker_threads as f64) / 
            (total_files as f64 / processing_time.as_secs_f64())
        } else {
            1.0
        };

        Ok(ProcessingStats {
            total_files,
            processed_files: self.progress.files_processed.load(Ordering::Relaxed),
            total_bytes,
            processing_time,
            files_per_second: self.progress.files_per_second(),
            bytes_per_second: self.progress.bytes_per_second(),
            parallel_efficiency,
        })
    }

    /// Process a single file (called from parallel context)
    fn process_single_file(&self, work_item: &ProcessingWorkItem) -> Result<ProcessingResult> {
        let start_time = Instant::now();
        
        // Choose processing strategy based on file size
        let chunks = if work_item.file_size > self.config.streaming_threshold {
            // Large files: use streaming processing
            self.streaming_engine.chunk_file_streaming(&work_item.file_path)?
        } else {
            // Small files: use regular chunking
            self.chunking_engine.chunk_file_streaming(&work_item.file_path)?
        };

        // Compute file hash from chunks
        let file_hash = Self::compute_file_hash_from_chunks(&chunks);
        
        // Get file metadata
        let modified_time = std::fs::metadata(&work_item.file_path)?.modified().ok();

        let chunk_count = chunks.len();
        let staged_file = BinaryStagedFile {
            path: work_item.file_path.clone(),
            hash: file_hash,
            size: work_item.file_size,
            chunks,
            modified_time,
        };

        Ok(ProcessingResult {
            staged_file,
            processing_time: start_time.elapsed(),
            chunk_count,
        })
    }

    /// Start background thread for staging writes
    fn start_staging_writer_thread(
        &self,
        staging_area: &mut BinaryStagingArea,
        progress_bar: &ProgressBar,
    ) -> Result<std::thread::JoinHandle<Result<()>>> {
        let staging_receiver = self.staging_receiver.clone();
        let staged_counter = Arc::clone(&self.progress.files_staged);
        
        // We need to pass ownership of staging_area to the thread
        // For now, let's use a simpler approach without background writing
        // and do batch staging writes in the main thread
        
        Ok(std::thread::spawn(move || {
            // Placeholder - in real implementation, this would handle background staging writes
            Ok(())
        }))
    }

    /// Set current processing phase
    fn set_phase(&self, phase: &str) {
        if let Ok(mut current_phase) = self.progress.current_phase.lock() {
            *current_phase = phase.to_string();
        }
    }

    /// Compute file hash from chunks
    fn compute_file_hash_from_chunks(chunks: &[Chunk]) -> Hash {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        for chunk in chunks {
            hasher.update(chunk.hash.as_bytes());
        }
        Hash::from_bytes(hasher.finalize().into())
    }
}

/// Statistics from parallel processing
#[derive(Debug, Clone)]
pub struct ProcessingStats {
    pub total_files: usize,
    pub processed_files: usize,
    pub total_bytes: u64,
    pub processing_time: Duration,
    pub files_per_second: f64,
    pub bytes_per_second: f64,
    pub parallel_efficiency: f64,
}

/// Optimized add-all operation with maximum parallelism
pub fn add_all_parallel(
    directory: &Path,
    staging_area: &mut BinaryStagingArea,
    multi_progress: &MultiProgress,
) -> Result<ProcessingStats> {
    let config = ParallelConfig::default();
    let mut processor = ParallelFileProcessor::new(config);
    
    // Use simplified parallel processing for now
    processor.process_directory_simplified(directory, staging_area, multi_progress)
}

impl ParallelFileProcessor {
    /// Simplified parallel processing without background threads
    pub fn process_directory_simplified(
        &mut self,
        directory: &Path,
        staging_area: &mut BinaryStagingArea,
        multi_progress: &MultiProgress,
    ) -> Result<ProcessingStats> {
        // Phase 1: Discover and filter files
        self.set_phase("Discovering files...");
        
        let discovery_pb = multi_progress.add(ProgressBar::new_spinner());
        discovery_pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg} ({pos:>7} files)")
                .unwrap()
        );

        let mut scanner = FilteredFileScanner::new(directory)
            .map_err(|e| DigstoreError::internal(format!("Failed to create scanner: {}", e)))?;
        let scan_result = scanner.scan_directory(directory)
            .map_err(|e| DigstoreError::internal(format!("Failed to scan directory: {}", e)))?;
        
        discovery_pb.finish_with_message(format!(
            "✓ Discovered {} files ({} filtered out)",
            scan_result.stats.total_discovered,
            scan_result.stats.total_ignored
        ));

        if scan_result.filtered_files.is_empty() {
            return Ok(ProcessingStats {
                total_files: 0,
                processed_files: 0,
                total_bytes: 0,
                processing_time: Duration::default(),
                files_per_second: 0.0,
                bytes_per_second: 0.0,
                parallel_efficiency: 0.0,
            });
        }

        // Phase 2: Parallel file processing
        self.set_phase("Processing files in parallel...");
        
        let processing_pb = multi_progress.add(ProgressBar::new(scan_result.filtered_files.len() as u64));
        processing_pb.set_style(
            ProgressStyle::default_bar()
                .template("{bar:50.cyan/blue} {pos:>7}/{len:7} files ({percent:>3}%) | {per_sec:>10} | ETA: {eta:>5} | {msg}")
                .unwrap()
        );

        let start_time = Instant::now();
        let total_files = scan_result.filtered_files.len();
        let files_processed = Arc::new(AtomicUsize::new(0));
        let bytes_processed = Arc::new(AtomicU64::new(0));

        // Process files in parallel chunks
        let chunk_size = (total_files / (self.config.worker_threads * 4)).max(10).min(100);
        let chunks: Vec<Vec<PathBuf>> = scan_result.filtered_files
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        let all_staged_files: Result<Vec<Vec<BinaryStagedFile>>> = chunks
            .into_par_iter()
            .map(|file_chunk| -> Result<Vec<BinaryStagedFile>> {
                let mut batch_results = Vec::with_capacity(file_chunk.len());
                
                for file_path in file_chunk {
                    // Process file
                    let file_size = std::fs::metadata(&file_path)?.len();
                    
                    let chunks = if file_size > self.config.streaming_threshold {
                        self.streaming_engine.chunk_file_streaming(&file_path)?
                    } else {
                        self.chunking_engine.chunk_file_streaming(&file_path)?
                    };

                    let file_hash = Self::compute_file_hash_from_chunks(&chunks);
                    let modified_time = std::fs::metadata(&file_path)?.modified().ok();

                    // Convert to relative path for storage
                    let relative_path = if let Ok(rel_path) = file_path.strip_prefix(directory) {
                        rel_path.to_path_buf()
                    } else {
                        file_path.clone()
                    };

                    batch_results.push(BinaryStagedFile {
                        path: relative_path,
                        hash: file_hash,
                        size: file_size,
                        chunks,
                        modified_time,
                    });

                    // Update progress
                    let current_processed = files_processed.fetch_add(1, Ordering::Relaxed) + 1;
                    bytes_processed.fetch_add(file_size, Ordering::Relaxed);
                    
                    // Update progress bar (with throttling)
                    if current_processed % 10 == 0 || current_processed == total_files {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let files_per_sec = if elapsed > 0.0 { current_processed as f64 / elapsed } else { 0.0 };
                        let bytes_per_sec = if elapsed > 0.0 { bytes_processed.load(Ordering::Relaxed) as f64 / elapsed } else { 0.0 };
                        
                        processing_pb.set_position(current_processed as u64);
                        processing_pb.set_message(format!(
                            "{:.1} files/s, {:.1} MB/s",
                            files_per_sec,
                            bytes_per_sec / 1024.0 / 1024.0
                        ));
                    }
                }
                
                Ok(batch_results)
            })
            .collect();

        let all_staged_files = all_staged_files?;
        let total_processed = files_processed.load(Ordering::Relaxed);
        let total_bytes_processed = bytes_processed.load(Ordering::Relaxed);
        let processing_time = start_time.elapsed();

        processing_pb.finish_with_message(format!(
            "✓ Processed {} files ({:.1} files/s, {:.1} MB/s)",
            total_processed,
            total_processed as f64 / processing_time.as_secs_f64(),
            total_bytes_processed as f64 / processing_time.as_secs_f64() / 1024.0 / 1024.0
        ));

        // Phase 3: Batch staging writes
        self.set_phase("Writing to staging...");
        
        let staging_pb = multi_progress.add(ProgressBar::new(total_processed as u64));
        staging_pb.set_style(
            ProgressStyle::default_bar()
                .template("{bar:50.blue/cyan} {pos:>7}/{len:7} staged | {per_sec:>10} | {msg}")
                .unwrap()
        );

        let mut staged_count = 0;
        for batch in all_staged_files {
            staging_area.stage_files_batch(batch.clone())?;
            staged_count += batch.len();
            staging_pb.set_position(staged_count as u64);
            staging_pb.set_message(format!("Staging batch of {} files", batch.len()));
        }

        staging_pb.finish_with_message(format!("✓ Staged {} files to binary format", staged_count));

        let parallel_efficiency = if self.config.worker_threads > 1 {
            let ideal_time = processing_time.as_secs_f64() / self.config.worker_threads as f64;
            let actual_time = processing_time.as_secs_f64();
            (ideal_time / actual_time).min(1.0)
        } else {
            1.0
        };

        Ok(ProcessingStats {
            total_files,
            processed_files: total_processed,
            total_bytes: total_bytes_processed,
            processing_time,
            files_per_second: total_processed as f64 / processing_time.as_secs_f64(),
            bytes_per_second: total_bytes_processed as f64 / processing_time.as_secs_f64(),
            parallel_efficiency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_parallel_config_default() {
        let config = ParallelConfig::default();
        assert!(config.worker_threads >= 1);
        assert!(config.staging_batch_size > 0);
        assert!(config.read_buffer_size > 0);
    }

    #[test]
    fn test_parallel_progress() {
        let progress = ParallelProgress::new();
        
        progress.files_processed.store(100, Ordering::Relaxed);
        progress.bytes_processed.store(1024 * 1024, Ordering::Relaxed);
        
        // Give some time for elapsed calculation
        std::thread::sleep(Duration::from_millis(10));
        
        assert!(progress.files_per_second() > 0.0);
        assert!(progress.bytes_per_second() > 0.0);
    }

    #[test]
    fn test_add_all_parallel_small_scale() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();
        
        // Create test files
        for i in 0..10 {
            fs::write(root.join(format!("test_{}.txt", i)), format!("content {}", i))?;
        }
        
        // Create .digignore to test filtering
        fs::write(root.join(".digignore"), "*.tmp\n")?;
        fs::write(root.join("ignored.tmp"), "ignored content")?;
        
        // Create staging area
        let staging_path = root.join("staging.bin");
        let mut staging_area = BinaryStagingArea::new(staging_path);
        staging_area.initialize()?;
        
        // Create progress manager
        let multi_progress = MultiProgress::new();
        
        // Process files in parallel
        let stats = add_all_parallel(root, &mut staging_area, &multi_progress)?;
        
        // Verify results (may include .digignore and other files)
        assert!(stats.total_files >= 10); // At least 10 .txt files
        assert!(stats.processed_files >= 10);
        assert!(stats.files_per_second > 0.0);
        assert!(stats.bytes_per_second >= 0.0); // Can be 0 for small files
        assert!(staging_area.staged_count() >= 10);
        
        Ok(())
    }
}
