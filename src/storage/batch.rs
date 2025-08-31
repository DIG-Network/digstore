//! Batch processing for efficient handling of many small files

use crate::core::{types::*, error::*};
use crate::storage::chunk::ChunkingEngine;
use crate::core::types::Chunk;
use std::path::{Path, PathBuf};
use sha2::Digest;
use rayon::prelude::*;
use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::sync::Arc;
use indicatif::ProgressBar;

/// Batch processor for efficiently handling many small files
pub struct BatchProcessor {
    batch_size: usize,
    worker_count: usize,
    chunk_dedup_cache: Arc<DashMap<Hash, ChunkInfo>>,
    performance_metrics: Arc<BatchMetrics>,
}

/// Information about a deduplicated chunk
#[derive(Clone, Debug)]
pub struct ChunkInfo {
    pub first_occurrence: PathBuf,
    pub reference_count: u32,
    pub size: u32,
}

/// Performance metrics for batch processing
pub struct BatchMetrics {
    pub files_processed: AtomicUsize,
    pub bytes_processed: AtomicU64,
    pub chunks_created: AtomicUsize,
    pub chunks_deduplicated: AtomicUsize,
    pub start_time: std::sync::Mutex<std::time::Instant>,
}

/// Result of processing a batch of files
pub struct BatchResult {
    pub file_entries: Vec<FileEntry>,
    pub chunks: Vec<Chunk>,
    pub deduplication_stats: DeduplicationStats,
    pub performance_metrics: PerformanceSnapshot,
}

/// Deduplication statistics
#[derive(Debug, Clone)]
pub struct DeduplicationStats {
    pub total_chunks: usize,
    pub unique_chunks: usize,
    pub bytes_saved: u64,
    pub deduplication_ratio: f64,
}

/// Performance snapshot
#[derive(Debug, Clone)]
pub struct PerformanceSnapshot {
    pub files_per_second: f64,
    pub mb_per_second: f64,
    pub chunks_per_second: f64,
    pub processing_time: std::time::Duration,
}

impl BatchProcessor {
    pub fn new() -> Self {
        Self {
            batch_size: 500, // Increase batch size for better throughput
            worker_count: std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4),
            chunk_dedup_cache: Arc::new(DashMap::new()),
            performance_metrics: Arc::new(BatchMetrics::new()),
        }
    }
    
    pub fn with_config(batch_size: usize, worker_count: usize) -> Self {
        Self {
            batch_size,
            worker_count,
            chunk_dedup_cache: Arc::new(DashMap::new()),
            performance_metrics: Arc::new(BatchMetrics::new()),
        }
    }
    
    /// Process many small files efficiently in parallel batches
    pub fn process_files_batch(
        &self,
        files: Vec<PathBuf>,
        progress: Option<&ProgressBar>,
    ) -> Result<BatchResult> {
        if files.is_empty() {
            return Ok(BatchResult::empty());
        }
        
        // Reset metrics at start of operation
        self.reset_metrics();
        
        if let Some(pb) = progress {
            pb.set_length(files.len() as u64);
            pb.set_message("Processing files in batches...");
        }
        
        // Process files in parallel batches (optimized for throughput)
        let batch_results: Result<Vec<_>> = files
            .par_chunks(self.batch_size.min(100)) // Smaller batches for better parallelism
            .enumerate()
            .map(|(batch_idx, batch)| {
                let batch_result = self.process_single_batch(batch, batch_idx)?;
                
                // Update progress less frequently to reduce overhead
                if let Some(pb) = progress {
                    if batch_idx % 10 == 0 {
                        pb.inc((batch.len() * 10) as u64);
                    }
                }
                
                Ok(batch_result)
            })
            .collect();
        
        let batch_results = batch_results?;
        
        // Combine all batch results
        let mut all_file_entries = Vec::new();
        let mut all_chunks = Vec::new();
        
        for batch_result in batch_results {
            all_file_entries.extend(batch_result.file_entries);
            all_chunks.extend(batch_result.chunks);
        }
        
        // Generate final statistics
        let dedup_stats = self.calculate_deduplication_stats();
        let perf_metrics = self.performance_metrics.snapshot();
        
        if let Some(pb) = progress {
            pb.finish_with_message("Batch processing complete");
        }
        
        Ok(BatchResult {
            file_entries: all_file_entries,
            chunks: all_chunks,
            deduplication_stats: dedup_stats,
            performance_metrics: perf_metrics,
        })
    }
    
    /// Process a single batch of files
    fn process_single_batch(&self, files: &[PathBuf], batch_idx: usize) -> Result<SingleBatchResult> {
        let chunking_engine = ChunkingEngine::new();
        let mut file_entries = Vec::with_capacity(files.len());
        let mut chunks = Vec::new();
        
        // Process files in this batch
        for file_path in files {
            match self.process_single_file_optimized(file_path, &chunking_engine) {
                Ok((file_entry, file_chunks)) => {
                    // Update metrics before moving file_entry
                    self.performance_metrics.files_processed.fetch_add(1, Ordering::Relaxed);
                    self.performance_metrics.bytes_processed.fetch_add(file_entry.size, Ordering::Relaxed);
                    
                    file_entries.push(file_entry);
                    chunks.extend(file_chunks);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to process {}: {}", file_path.display(), e);
                }
            }
        }
        
        Ok(SingleBatchResult {
            file_entries,
            chunks,
        })
    }
    
    /// Process a single file optimized for small files
    fn process_single_file_optimized(
        &self,
        file_path: &Path,
        chunking_engine: &ChunkingEngine,
    ) -> Result<(FileEntry, Vec<Chunk>)> {
        let file_size = std::fs::metadata(file_path)?.len();
        
        // Optimize for small files (increased threshold for better performance)
        if file_size <= 16384 { // 16KB threshold
            // Small files: single chunk, no CDC overhead
            return self.process_tiny_file(file_path, file_size);
        }
        
        // Medium files: use streaming chunking
        let chunks = chunking_engine.chunk_file_streaming(file_path)?;
        
        // Process chunks for deduplication
        let processed_chunks = self.process_chunks_for_deduplication(chunks, file_path);
        
        // Create file entry
        let file_hash = Self::compute_file_hash_from_chunks(&processed_chunks);
        let file_entry = FileEntry {
            path: file_path.to_path_buf(),
            hash: file_hash,
            size: file_size,
            chunks: processed_chunks.iter().map(|c| ChunkRef {
                hash: c.hash,
                offset: c.offset,
                size: c.size,
            }).collect(),
            metadata: FileMetadata {
                mode: 0o644,
                modified: std::fs::metadata(file_path)?.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };
        
        Ok((file_entry, processed_chunks))
    }
    
    /// Process very small files as single chunks
    fn process_tiny_file(&self, file_path: &Path, file_size: u64) -> Result<(FileEntry, Vec<Chunk>)> {
        // Read small file directly
        let data = std::fs::read(file_path)?;
        let hash = crate::core::hash::sha256(&data);
        
        let chunk = Chunk {
            hash,
            offset: 0,
            size: data.len() as u32,
            data,
        };
        
        // Process for deduplication
        let processed_chunks = self.process_chunks_for_deduplication(vec![chunk], file_path);
        
        let file_entry = FileEntry {
            path: file_path.to_path_buf(),
            hash,
            size: file_size,
            chunks: vec![ChunkRef {
                hash,
                offset: 0,
                size: file_size as u32,
            }],
            metadata: FileMetadata {
                mode: 0o644,
                modified: std::fs::metadata(file_path)?.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };
        
        Ok((file_entry, processed_chunks))
    }
    
    /// Process chunks for deduplication tracking
    fn process_chunks_for_deduplication(&self, chunks: Vec<Chunk>, file_path: &Path) -> Vec<Chunk> {
        let mut result_chunks = Vec::new();
        
        for chunk in chunks {
            self.performance_metrics.chunks_created.fetch_add(1, Ordering::Relaxed);
            
            match self.chunk_dedup_cache.entry(chunk.hash) {
                dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                    // Chunk already exists - deduplicated!
                    entry.get_mut().reference_count += 1;
                    self.performance_metrics.chunks_deduplicated.fetch_add(1, Ordering::Relaxed);
                    
                    // Still add to result (needed for file reconstruction)
                    result_chunks.push(chunk);
                }
                dashmap::mapref::entry::Entry::Vacant(entry) => {
                    // New chunk
                    entry.insert(ChunkInfo {
                        first_occurrence: file_path.to_path_buf(),
                        reference_count: 1,
                        size: chunk.size,
                    });
                    
                    result_chunks.push(chunk);
                }
            }
        }
        
        result_chunks
    }
    
    fn calculate_deduplication_stats(&self) -> DeduplicationStats {
        let total_chunks = self.performance_metrics.chunks_created.load(Ordering::Relaxed);
        let unique_chunks = self.chunk_dedup_cache.len();
        let deduplicated_chunks = self.performance_metrics.chunks_deduplicated.load(Ordering::Relaxed);
        
        let bytes_saved: u64 = self.chunk_dedup_cache
            .iter()
            .map(|entry| {
                let chunk_info = entry.value();
                (chunk_info.reference_count.saturating_sub(1) as u64) * (chunk_info.size as u64)
            })
            .sum();
        
        let deduplication_ratio = if total_chunks > 0 {
            deduplicated_chunks as f64 / total_chunks as f64
        } else {
            0.0
        };
        
        DeduplicationStats {
            total_chunks,
            unique_chunks,
            bytes_saved,
            deduplication_ratio,
        }
    }
    
    fn compute_file_hash_from_chunks(chunks: &[Chunk]) -> Hash {
        let mut hasher = sha2::Sha256::new();
        for chunk in chunks {
            hasher.update(&chunk.data);
        }
        Hash::from_bytes(hasher.finalize().into())
    }
    
    /// Get current performance metrics
    pub fn get_performance_metrics(&self) -> PerformanceSnapshot {
        self.performance_metrics.snapshot()
    }
    
    /// Reset performance metrics
    pub fn reset_metrics(&self) {
        // Clear deduplication cache
        self.chunk_dedup_cache.clear();
        
        // Reset atomic counters
        self.performance_metrics.files_processed.store(0, Ordering::Relaxed);
        self.performance_metrics.bytes_processed.store(0, Ordering::Relaxed);
        self.performance_metrics.chunks_created.store(0, Ordering::Relaxed);
        self.performance_metrics.chunks_deduplicated.store(0, Ordering::Relaxed);
        *self.performance_metrics.start_time.lock().unwrap() = std::time::Instant::now();
    }
}

/// Result of processing a single batch
struct SingleBatchResult {
    file_entries: Vec<FileEntry>,
    chunks: Vec<Chunk>,
}

impl BatchMetrics {
    fn new() -> Self {
        Self {
            files_processed: AtomicUsize::new(0),
            bytes_processed: AtomicU64::new(0),
            chunks_created: AtomicUsize::new(0),
            chunks_deduplicated: AtomicUsize::new(0),
            start_time: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }
    
    fn snapshot(&self) -> PerformanceSnapshot {
        let elapsed = self.start_time.lock().unwrap().elapsed();
        let files = self.files_processed.load(Ordering::Relaxed);
        let bytes = self.bytes_processed.load(Ordering::Relaxed);
        let chunks = self.chunks_created.load(Ordering::Relaxed);
        
        PerformanceSnapshot {
            files_per_second: files as f64 / elapsed.as_secs_f64(),
            mb_per_second: bytes as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0),
            chunks_per_second: chunks as f64 / elapsed.as_secs_f64(),
            processing_time: elapsed,
        }
    }
    

}

impl BatchResult {
    fn empty() -> Self {
        Self {
            file_entries: Vec::new(),
            chunks: Vec::new(),
            deduplication_stats: DeduplicationStats {
                total_chunks: 0,
                unique_chunks: 0,
                bytes_saved: 0,
                deduplication_ratio: 0.0,
            },
            performance_metrics: PerformanceSnapshot {
                files_per_second: 0.0,
                mb_per_second: 0.0,
                chunks_per_second: 0.0,
                processing_time: std::time::Duration::from_secs(0),
            },
        }
    }
}

/// Optimized file scanner for large numbers of small files
pub struct OptimizedFileScanner {
    ignore_patterns: Vec<String>,
    max_file_size: u64,
    min_file_size: u64,
}

impl OptimizedFileScanner {
    pub fn new() -> Self {
        Self {
            ignore_patterns: vec![
                ".git".to_string(),
                ".layerstore".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                ".DS_Store".to_string(),
            ],
            max_file_size: 100 * 1024 * 1024, // 100MB
            min_file_size: 0,
        }
    }
    
    /// Scan directory in parallel for many small files
    pub fn scan_directory_parallel(&self, root: &Path) -> Result<Vec<PathBuf>> {
        use walkdir::WalkDir;
        
        let files: Vec<PathBuf> = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .par_bridge() // Convert to parallel iterator
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.path().to_path_buf())
            .filter(|path| self.should_include_file(path))
            .collect();
        
        Ok(files)
    }
    
    fn should_include_file(&self, path: &Path) -> bool {
        // Quick size check
        if let Ok(metadata) = path.metadata() {
            let size = metadata.len();
            if size < self.min_file_size || size > self.max_file_size {
                return false;
            }
        }
        
        // Pattern matching
        let path_str = path.to_string_lossy();
        for pattern in &self.ignore_patterns {
            if path_str.contains(pattern) {
                return false;
            }
        }
        
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    use std::io::Write;

    #[test]
    fn test_batch_processor_small_files() {
        let temp_dir = TempDir::new().unwrap();
        let mut files = Vec::new();
        
        // Create 100 small test files
        for i in 0..100 {
            let file_path = temp_dir.path().join(format!("test_{}.txt", i));
            let content = format!("Test file content {}", i);
            fs::write(&file_path, content).unwrap();
            files.push(file_path);
        }
        
        let processor = BatchProcessor::new();
        let result = processor.process_files_batch(files, None).unwrap();
        
        assert_eq!(result.file_entries.len(), 100);
        assert!(!result.chunks.is_empty());
        
        // Check performance metrics
        let metrics = result.performance_metrics;
        assert!(metrics.files_per_second > 0.0);
        assert!(metrics.processing_time.as_secs() < 10); // Should be fast
    }
    
    #[test]
    fn test_deduplication_with_identical_files() {
        let temp_dir = TempDir::new().unwrap();
        let mut files = Vec::new();
        
        // Create 50 identical files
        let content = "This is identical content for deduplication testing";
        for i in 0..50 {
            let file_path = temp_dir.path().join(format!("identical_{}.txt", i));
            fs::write(&file_path, content).unwrap();
            files.push(file_path);
        }
        
        let processor = BatchProcessor::new();
        let result = processor.process_files_batch(files, None).unwrap();
        
        assert_eq!(result.file_entries.len(), 50);
        
        // Should have high deduplication ratio
        assert!(result.deduplication_stats.deduplication_ratio > 0.9);
        assert!(result.deduplication_stats.bytes_saved > 0);
    }
    
    #[test]
    fn test_optimized_file_scanner() {
        let temp_dir = TempDir::new().unwrap();
        
        // Create test directory structure
        fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        fs::create_dir_all(temp_dir.path().join("target")).unwrap();
        
        // Create files
        fs::write(temp_dir.path().join("README.md"), "readme").unwrap();
        fs::write(temp_dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(temp_dir.path().join(".git/config"), "git config").unwrap();
        fs::write(temp_dir.path().join("target/debug"), "binary").unwrap();
        
        let scanner = OptimizedFileScanner::new();
        let files = scanner.scan_directory_parallel(temp_dir.path()).unwrap();
        
        // Should include README.md and src/main.rs but exclude .git and target
        assert!(files.iter().any(|p| p.file_name().unwrap() == "README.md"));
        assert!(files.iter().any(|p| p.file_name().unwrap() == "main.rs"));
        assert!(!files.iter().any(|p| p.to_string_lossy().contains(".git")));
        assert!(!files.iter().any(|p| p.to_string_lossy().contains("target")));
    }
    
    #[test]
    fn test_performance_metrics() {
        let metrics = BatchMetrics::new();
        
        // Simulate processing
        metrics.files_processed.store(1000, Ordering::Relaxed);
        metrics.bytes_processed.store(50 * 1024 * 1024, Ordering::Relaxed); // 50MB
        
        let snapshot = metrics.snapshot();
        
        assert!(snapshot.files_per_second > 0.0);
        assert!(snapshot.mb_per_second > 0.0);
        assert!(snapshot.processing_time.as_nanos() > 0);
    }
}
