# Small File Optimization Guide

## Overview

Efficiently handling large numbers of small files (20,000+) requires specialized optimization strategies. This guide covers the specific techniques needed to achieve high performance when committing thousands of small files.

## Performance Challenges with Small Files

### 1. I/O Overhead
- **System call overhead**: Each file operation involves kernel calls
- **Metadata operations**: stat(), open(), read(), close() for each file
- **Directory traversal**: Walking large directory trees
- **Disk seek time**: Random access patterns for many small files

### 2. Memory Fragmentation
- **Many small allocations**: Each file creates small memory allocations
- **Staging area growth**: HashMap with thousands of entries
- **Chunk overhead**: Small files create many small chunks
- **Index bloat**: File index grows linearly with file count

### 3. Progress Feedback Challenges
- **Update frequency**: Updating progress for each file is expensive
- **Terminal output**: Too many updates can slow down processing
- **Batching required**: Group operations for efficiency

## Optimization Strategies

### 1. Batch Processing Architecture
```rust
pub struct BatchFileProcessor {
    batch_size: usize,
    worker_threads: usize,
    staging_buffer: Vec<FileEntry>,
}

impl BatchFileProcessor {
    pub fn new() -> Self {
        Self {
            batch_size: 100,  // Process 100 files at a time
            worker_threads: num_cpus::get(),
            staging_buffer: Vec::with_capacity(1000),
        }
    }
    
    pub async fn process_files_in_batches(
        &mut self,
        files: Vec<PathBuf>,
        progress: &ProgressBar,
    ) -> Result<Vec<FileEntry>> {
        let mut results = Vec::with_capacity(files.len());
        
        for batch in files.chunks(self.batch_size) {
            // Process batch in parallel
            let batch_results = self.process_batch_parallel(batch).await?;
            results.extend(batch_results);
            
            // Update progress once per batch (not per file)
            progress.inc(batch.len() as u64);
            
            // Yield to prevent blocking
            tokio::task::yield_now().await;
        }
        
        Ok(results)
    }
    
    async fn process_batch_parallel(&self, batch: &[PathBuf]) -> Result<Vec<FileEntry>> {
        use rayon::prelude::*;
        
        batch
            .par_iter()
            .map(|path| self.process_single_file(path))
            .collect::<Result<Vec<_>>>()
    }
}
```

### 2. Optimized File Scanning
```rust
use walkdir::WalkDir;
use std::sync::mpsc;

pub struct OptimizedFileScanner {
    ignore_patterns: Vec<String>,
    max_file_size: u64,
    min_file_size: u64,
}

impl OptimizedFileScanner {
    pub fn scan_directory_parallel(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let (sender, receiver) = mpsc::channel();
        let sender = Arc::new(Mutex::new(sender));
        
        // Use multiple threads for directory traversal
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .par_bridge() // Convert to parallel iterator
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .for_each(|entry| {
                let path = entry.path().to_path_buf();
                
                // Apply filters
                if self.should_include_file(&path) {
                    sender.lock().unwrap().send(path).unwrap();
                }
            });
        
        // Collect all paths
        drop(sender); // Close channel
        let mut files = Vec::new();
        while let Ok(path) = receiver.recv() {
            files.push(path);
        }
        
        Ok(files)
    }
    
    fn should_include_file(&self, path: &Path) -> bool {
        // Quick size check without full metadata
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
```

### 3. Bulk Staging Operations
```rust
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct BulkStagingOperation {
    files: Vec<StagedFile>,
    operation_id: uuid::Uuid,
    timestamp: i64,
}

pub struct BulkStagingManager {
    staging_dir: PathBuf,
    batch_size: usize,
    pending_operations: Vec<BulkStagingOperation>,
}

impl BulkStagingManager {
    pub fn stage_files_bulk(&mut self, files: Vec<StagedFile>) -> Result<()> {
        // Group files into batches
        for batch in files.chunks(self.batch_size) {
            let operation = BulkStagingOperation {
                files: batch.to_vec(),
                operation_id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now().timestamp(),
            };
            
            // Write batch to temporary file
            let temp_file = self.staging_dir.join(format!("batch_{}.tmp", operation.operation_id));
            let serialized = bincode::serialize(&operation)?;
            std::fs::write(&temp_file, serialized)?;
            
            self.pending_operations.push(operation);
        }
        
        Ok(())
    }
    
    pub fn commit_all_batches(&mut self) -> Result<Vec<StagedFile>> {
        let mut all_files = Vec::new();
        
        // Process all pending batches
        for operation in &self.pending_operations {
            all_files.extend(operation.files.clone());
            
            // Remove temporary file
            let temp_file = self.staging_dir.join(format!("batch_{}.tmp", operation.operation_id));
            std::fs::remove_file(temp_file).ok(); // Ignore errors
        }
        
        self.pending_operations.clear();
        Ok(all_files)
    }
}
```

### 4. Optimized Chunk Deduplication
```rust
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ChunkDeduplicator {
    chunk_map: DashMap<Hash, ChunkInfo>,
    dedup_stats: AtomicU64,
    total_chunks: AtomicU64,
}

#[derive(Clone)]
pub struct ChunkInfo {
    first_occurrence: PathBuf,
    reference_count: u32,
    size: u32,
}

impl ChunkDeduplicator {
    pub fn process_chunk(&self, chunk: Chunk, file_path: &Path) -> ChunkProcessResult {
        self.total_chunks.fetch_add(1, Ordering::Relaxed);
        
        match self.chunk_map.entry(chunk.hash) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                // Chunk already exists - deduplicated!
                entry.get_mut().reference_count += 1;
                self.dedup_stats.fetch_add(chunk.size as u64, Ordering::Relaxed);
                
                ChunkProcessResult::Deduplicated {
                    hash: chunk.hash,
                    original_file: entry.get().first_occurrence.clone(),
                }
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                // New chunk
                entry.insert(ChunkInfo {
                    first_occurrence: file_path.to_path_buf(),
                    reference_count: 1,
                    size: chunk.size,
                });
                
                ChunkProcessResult::New(chunk)
            }
        }
    }
    
    pub fn get_deduplication_stats(&self) -> DeduplicationStats {
        let total_chunks = self.total_chunks.load(Ordering::Relaxed);
        let dedup_bytes = self.dedup_stats.load(Ordering::Relaxed);
        let unique_chunks = self.chunk_map.len() as u64;
        
        DeduplicationStats {
            total_chunks,
            unique_chunks,
            deduplication_ratio: (total_chunks - unique_chunks) as f64 / total_chunks as f64,
            bytes_saved: dedup_bytes,
        }
    }
}
```

### 5. Efficient Progress Updates
```rust
use std::time::{Duration, Instant};

pub struct ThrottledProgress {
    progress_bar: ProgressBar,
    last_update: Instant,
    update_interval: Duration,
    pending_increment: u64,
}

impl ThrottledProgress {
    pub fn new(total: u64, message: &str) -> Self {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} {msg} [{bar:40}] {pos}/{len} ({per_sec}) {eta}")
                .unwrap()
        );
        pb.set_message(message.to_string());
        
        Self {
            progress_bar: pb,
            last_update: Instant::now(),
            update_interval: Duration::from_millis(100), // Update at most 10 times per second
            pending_increment: 0,
        }
    }
    
    pub fn inc(&mut self, delta: u64) {
        self.pending_increment += delta;
        
        // Only update display if enough time has passed
        if self.last_update.elapsed() >= self.update_interval {
            self.progress_bar.inc(self.pending_increment);
            self.pending_increment = 0;
            self.last_update = Instant::now();
        }
    }
    
    pub fn finish(&mut self) {
        // Apply any pending increments
        if self.pending_increment > 0 {
            self.progress_bar.inc(self.pending_increment);
        }
        self.progress_bar.finish();
    }
}
```

## Memory-Efficient Data Structures

### 1. Compact File Entries
```rust
use compact_str::CompactString;

#[derive(Clone)]
pub struct CompactFileEntry {
    // Use CompactString for small paths (avoids heap allocation for short paths)
    path: CompactString,
    hash: Hash,
    size: u32,  // Use u32 for file size if files are <4GB
    chunk_count: u16,  // Most files have <65k chunks
    first_chunk_index: u32,
}

impl CompactFileEntry {
    pub fn new(path: &Path, hash: Hash, size: u64, chunks: &[ChunkRef]) -> Self {
        Self {
            path: CompactString::new(path.to_string_lossy()),
            hash,
            size: size as u32,  // Truncate for small files
            chunk_count: chunks.len() as u16,
            first_chunk_index: 0,  // Set when adding to layer
        }
    }
}
```

### 2. Chunk Reference Optimization
```rust
#[repr(C, packed)]
pub struct PackedChunkRef {
    hash: [u8; 32],     // 32 bytes
    offset: u32,        // 4 bytes (supports files up to 4GB)
    size: u16,          // 2 bytes (supports chunks up to 64KB)
    _padding: [u8; 2],  // 2 bytes padding for alignment
}
// Total: 40 bytes per chunk reference (vs 48+ for full struct)

impl PackedChunkRef {
    pub fn new(hash: Hash, offset: u64, size: u32) -> Self {
        Self {
            hash: *hash.as_bytes(),
            offset: offset as u32,
            size: size as u16,
            _padding: [0; 2],
        }
    }
}
```

### 3. Efficient Staging Area
```rust
use indexmap::IndexMap;
use ahash::AHashMap;

pub struct OptimizedStagingArea {
    // Use IndexMap for insertion order + fast lookups
    files: IndexMap<CompactString, StagedFile, ahash::RandomState>,
    // Separate index for hash-based lookups
    hash_index: AHashMap<Hash, CompactString>,
    // Track dirty state for batch persistence
    dirty_files: HashSet<CompactString>,
    last_persist: Instant,
    persist_interval: Duration,
}

impl OptimizedStagingArea {
    pub fn add_files_optimized(&mut self, files: Vec<(PathBuf, StagedFile)>) -> Result<()> {
        // Reserve capacity to avoid reallocations
        self.files.reserve(files.len());
        self.hash_index.reserve(files.len());
        
        for (path, staged_file) in files {
            let path_key = CompactString::new(path.to_string_lossy());
            
            // Update indices
            self.hash_index.insert(staged_file.file_entry.hash, path_key.clone());
            self.files.insert(path_key.clone(), staged_file);
            self.dirty_files.insert(path_key);
        }
        
        // Batch persistence
        if self.last_persist.elapsed() > self.persist_interval || self.dirty_files.len() > 1000 {
            self.persist_dirty_files()?;
        }
        
        Ok(())
    }
    
    fn persist_dirty_files(&mut self) -> Result<()> {
        if self.dirty_files.is_empty() {
            return Ok(());
        }
        
        // Create a compact representation for persistence
        let dirty_entries: Vec<_> = self.dirty_files
            .iter()
            .filter_map(|path| self.files.get(path).map(|f| (path.clone(), f.clone())))
            .collect();
        
        // Serialize and write in single operation
        let serialized = bincode::serialize(&dirty_entries)?;
        std::fs::write(self.get_staging_file_path(), serialized)?;
        
        self.dirty_files.clear();
        self.last_persist = Instant::now();
        
        Ok(())
    }
}
```

## Parallel Processing Pipeline

### 1. Multi-Stage Pipeline
```rust
use crossbeam::channel::{bounded, Receiver, Sender};
use std::thread;

pub struct SmallFilesPipeline {
    file_scanner: FileScanner,
    chunk_processor: ChunkProcessor,
    deduplicator: ChunkDeduplicator,
    staging_manager: StagingManager,
}

impl SmallFilesPipeline {
    pub fn process_directory(&self, root: &Path, progress: &MultiStageProgress) -> Result<CommitResult> {
        // Stage 1: Parallel file discovery
        let (file_tx, file_rx) = bounded(1000);
        let (chunk_tx, chunk_rx) = bounded(10000);
        let (dedup_tx, dedup_rx) = bounded(10000);
        
        // File scanning thread
        let scan_handle = {
            let file_tx = file_tx.clone();
            let root = root.to_path_buf();
            thread::spawn(move || {
                self.file_scanner.scan_parallel(&root, file_tx)
            })
        };
        
        // Chunk processing threads (multiple workers)
        let chunk_handles: Vec<_> = (0..num_cpus::get())
            .map(|_| {
                let file_rx = file_rx.clone();
                let chunk_tx = chunk_tx.clone();
                thread::spawn(move || {
                    while let Ok(file_path) = file_rx.recv() {
                        match self.chunk_processor.process_file(&file_path) {
                            Ok(chunks) => {
                                for chunk in chunks {
                                    chunk_tx.send((file_path.clone(), chunk)).unwrap();
                                }
                            }
                            Err(e) => {
                                eprintln!("Error processing {}: {}", file_path.display(), e);
                            }
                        }
                    }
                })
            })
            .collect();
        
        // Deduplication thread
        let dedup_handle = {
            let chunk_rx = chunk_rx.clone();
            let dedup_tx = dedup_tx.clone();
            thread::spawn(move || {
                while let Ok((file_path, chunk)) = chunk_rx.recv() {
                    let result = self.deduplicator.process_chunk(chunk, &file_path);
                    dedup_tx.send((file_path, result)).unwrap();
                }
            })
        };
        
        // Collection and staging thread
        let staging_handle = {
            let dedup_rx = dedup_rx.clone();
            thread::spawn(move || {
                let mut file_entries = HashMap::new();
                
                while let Ok((file_path, chunk_result)) = dedup_rx.recv() {
                    file_entries
                        .entry(file_path)
                        .or_insert_with(Vec::new)
                        .push(chunk_result);
                }
                
                // Convert to staged files
                let staged_files: Vec<_> = file_entries
                    .into_iter()
                    .map(|(path, chunks)| self.create_staged_file(path, chunks))
                    .collect::<Result<Vec<_>>>()?;
                
                Ok(staged_files)
            })
        };
        
        // Wait for completion and collect results
        scan_handle.join().unwrap()?;
        for handle in chunk_handles {
            handle.join().unwrap();
        }
        dedup_handle.join().unwrap();
        let staged_files = staging_handle.join().unwrap()?;
        
        Ok(CommitResult { staged_files })
    }
}
```

### 2. Lock-Free Concurrent Processing
```rust
use crossbeam::utils::Backoff;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LockFreeFileProcessor {
    processed_count: AtomicUsize,
    error_count: AtomicUsize,
    total_bytes: AtomicU64,
}

impl LockFreeFileProcessor {
    pub fn process_files_lockfree(
        &self,
        files: &[PathBuf],
        results: &DashMap<PathBuf, FileEntry>,
    ) -> Result<()> {
        files.par_iter().for_each(|path| {
            match self.process_single_file_optimized(path) {
                Ok(file_entry) => {
                    self.total_bytes.fetch_add(file_entry.size, Ordering::Relaxed);
                    results.insert(path.clone(), file_entry);
                    self.processed_count.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        
        Ok(())
    }
    
    fn process_single_file_optimized(&self, path: &Path) -> Result<FileEntry> {
        // Optimized for small files
        let data = std::fs::read(path)?;
        
        if data.len() <= 4096 {
            // Small files: single chunk, no CDC needed
            let hash = sha256(&data);
            Ok(FileEntry {
                path: path.to_path_buf(),
                hash,
                size: data.len() as u64,
                chunks: vec![ChunkRef {
                    hash,
                    offset: 0,
                    size: data.len() as u32,
                }],
            })
        } else {
            // Larger files: use content-defined chunking
            self.chunk_file_cdc(&data, path)
        }
    }
}
```

## I/O Optimization Strategies

### 1. Batch File Reading
```rust
use std::io::{BufReader, Read};

pub struct BatchFileReader {
    buffer_pool: BufferPool,
    read_ahead_size: usize,
}

impl BatchFileReader {
    pub fn read_files_batch(&self, files: &[PathBuf]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        // Group small files for efficient I/O
        let small_files: Vec<_> = files.iter()
            .filter(|path| {
                path.metadata().map(|m| m.len() < 64 * 1024).unwrap_or(false)
            })
            .collect();
        
        // Read small files in batches to reduce syscall overhead
        let mut results = Vec::with_capacity(files.len());
        
        for batch in small_files.chunks(50) {
            let batch_results = self.read_batch_sequential(batch)?;
            results.extend(batch_results);
        }
        
        Ok(results)
    }
    
    fn read_batch_sequential(&self, files: &[&PathBuf]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut results = Vec::new();
        let mut buffer = self.buffer_pool.get_buffer();
        
        for &path in files {
            let mut file = File::open(path)?;
            buffer.clear();
            file.read_to_end(&mut buffer)?;
            
            results.push((path.clone(), buffer.clone()));
        }
        
        Ok(results)
    }
}
```

### 2. Asynchronous File Operations
```rust
use tokio::fs;
use futures::stream::{FuturesUnordered, StreamExt};

pub struct AsyncFileProcessor {
    max_concurrent: usize,
}

impl AsyncFileProcessor {
    pub async fn process_files_async(&self, files: Vec<PathBuf>) -> Result<Vec<FileEntry>> {
        let mut futures = FuturesUnordered::new();
        let mut results = Vec::with_capacity(files.len());
        
        for batch in files.chunks(self.max_concurrent) {
            // Process batch concurrently
            for path in batch {
                futures.push(self.process_file_async(path.clone()));
            }
            
            // Collect results as they complete
            while let Some(result) = futures.next().await {
                results.push(result?);
            }
        }
        
        Ok(results)
    }
    
    async fn process_file_async(&self, path: PathBuf) -> Result<FileEntry> {
        let data = fs::read(&path).await?;
        
        // Process in background thread pool to avoid blocking async runtime
        let chunks = tokio::task::spawn_blocking(move || {
            chunk_data(&data)
        }).await??;
        
        Ok(FileEntry::new(path, data.len() as u64, chunks))
    }
}
```

## Performance Monitoring and Tuning

### 1. Real-time Performance Metrics
```rust
pub struct PerformanceMonitor {
    start_time: Instant,
    last_checkpoint: Instant,
    files_processed: AtomicUsize,
    bytes_processed: AtomicU64,
    chunks_created: AtomicUsize,
}

impl PerformanceMonitor {
    pub fn record_file_processed(&self, size: u64, chunk_count: usize) {
        self.files_processed.fetch_add(1, Ordering::Relaxed);
        self.bytes_processed.fetch_add(size, Ordering::Relaxed);
        self.chunks_created.fetch_add(chunk_count, Ordering::Relaxed);
    }
    
    pub fn get_current_metrics(&self) -> CurrentMetrics {
        let elapsed = self.start_time.elapsed();
        let files = self.files_processed.load(Ordering::Relaxed);
        let bytes = self.bytes_processed.load(Ordering::Relaxed);
        let chunks = self.chunks_created.load(Ordering::Relaxed);
        
        CurrentMetrics {
            files_per_second: files as f64 / elapsed.as_secs_f64(),
            mb_per_second: bytes as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0),
            chunks_per_second: chunks as f64 / elapsed.as_secs_f64(),
            total_files: files,
            total_bytes: bytes,
            elapsed,
        }
    }
}
```

### 2. Adaptive Performance Tuning
```rust
pub struct AdaptiveProcessor {
    current_batch_size: AtomicUsize,
    performance_history: Mutex<VecDeque<PerformanceSample>>,
    min_batch_size: usize,
    max_batch_size: usize,
}

impl AdaptiveProcessor {
    pub fn adjust_batch_size(&self, current_performance: f64) {
        let mut history = self.performance_history.lock().unwrap();
        history.push_back(PerformanceSample {
            batch_size: self.current_batch_size.load(Ordering::Relaxed),
            files_per_second: current_performance,
            timestamp: Instant::now(),
        });
        
        // Keep only recent samples
        while history.len() > 10 {
            history.pop_front();
        }
        
        // Adjust batch size based on performance trend
        if let Some(best_sample) = history.iter().max_by(|a, b| {
            a.files_per_second.partial_cmp(&b.files_per_second).unwrap()
        }) {
            let current_batch = self.current_batch_size.load(Ordering::Relaxed);
            let target_batch = if best_sample.files_per_second > current_performance {
                // Performance is declining, try the best batch size
                best_sample.batch_size
            } else {
                // Performance is good, try increasing batch size
                (current_batch * 110 / 100).min(self.max_batch_size)
            };
            
            self.current_batch_size.store(target_batch, Ordering::Relaxed);
        }
    }
}
```

## Configuration for Different Workloads

### 1. Small Files Optimized Config
```toml
[small_files]
# Optimize for many small files
batch_size = 200
worker_threads = 8  # CPU cores
chunk_threshold = "4KB"  # Don't chunk very small files
staging_persist_interval = "5s"
progress_update_interval = "200ms"

[chunking.small_files]
min_size = "1KB"
avg_size = "4KB"
max_size = "16KB"
skip_chunking_threshold = "4KB"  # Single chunk for small files
```

### 2. Large Files Optimized Config
```toml
[large_files]
# Optimize for large files
streaming_threshold = "100MB"
memory_map_threshold = "10MB"
chunk_size = "1MB"
buffer_size = "64KB"
max_memory_usage = "2GB"

[chunking.large_files]
min_size = "512KB"
avg_size = "1MB"
max_size = "4MB"
parallel_chunking = true
```

### 3. Mixed Workload Config
```toml
[adaptive]
# Automatically adjust based on file sizes
auto_detect_workload = true
small_file_threshold = "64KB"
large_file_threshold = "100MB"
batch_size_min = 50
batch_size_max = 500
performance_monitoring = true
```

## Testing and Validation

### 1. Performance Test Suite
```rust
#[cfg(test)]
mod performance_tests {
    use super::*;
    
    #[test]
    fn test_20k_small_files_performance() {
        let temp_dir = create_test_files(20000, 1024..8192);
        let start = Instant::now();
        
        let mut store = Store::init(&temp_dir)?;
        store.add_directory(&temp_dir, true)?;
        let commit_id = store.commit("20k files test")?;
        
        let duration = start.elapsed();
        
        // Performance requirements
        assert!(duration.as_secs() < 60, "Should complete in <60 seconds");
        
        let files_per_sec = 20000.0 / duration.as_secs_f64();
        assert!(files_per_sec > 300.0, "Should process >300 files/second");
        
        // Memory usage should be reasonable
        let memory_mb = get_peak_memory_usage() / (1024 * 1024);
        assert!(memory_mb < 500, "Should use <500MB memory");
    }
    
    #[test]
    fn test_large_file_streaming() {
        let large_file = create_sparse_file(5 * 1024 * 1024 * 1024); // 5GB
        let start = Instant::now();
        
        let chunks = chunk_large_file_streaming(&large_file)?;
        
        let duration = start.elapsed();
        let throughput_mb_s = (5 * 1024) as f64 / duration.as_secs_f64();
        
        assert!(throughput_mb_s > 500.0, "Should achieve >500 MB/s throughput");
        
        // Memory should remain constant
        let memory_mb = get_peak_memory_usage() / (1024 * 1024);
        assert!(memory_mb < 200, "Should use <200MB memory regardless of file size");
    }
}
```

### 2. Stress Testing
```rust
#[test]
fn stress_test_mixed_workload() {
    // Create mixed workload: 10k small files + 5 large files
    let temp_dir = TempDir::new()?;
    
    // 10,000 small files (1-10KB)
    create_small_files(&temp_dir, 10000, 1024..10240)?;
    
    // 5 large files (100MB each)
    create_large_files(&temp_dir, 5, 100 * 1024 * 1024)?;
    
    let start = Instant::now();
    
    let mut store = Store::init(&temp_dir)?;
    store.add_directory(&temp_dir, true)?;
    let commit_id = store.commit("Mixed workload test")?;
    
    let duration = start.elapsed();
    
    // Should handle mixed workload efficiently
    assert!(duration.as_secs() < 120, "Mixed workload should complete in <2 minutes");
    
    // Verify all files are accessible
    let file_count = count_files_in_commit(&store, commit_id)?;
    assert_eq!(file_count, 10005, "All files should be committed");
}
```

## Monitoring and Diagnostics

### 1. Performance Dashboard
```rust
pub struct PerformanceDashboard {
    metrics: Arc<PerformanceMonitor>,
    update_interval: Duration,
}

impl PerformanceDashboard {
    pub fn display_real_time_metrics(&self) {
        let mut last_update = Instant::now();
        
        loop {
            if last_update.elapsed() >= self.update_interval {
                let metrics = self.metrics.get_current_metrics();
                
                println!("\r{} Files: {}/s | {} Bytes: {:.1} MB/s | {} Chunks: {}/s | Memory: {} MB",
                    "📊".green(),
                    metrics.files_per_second as u32,
                    "💾".blue(),
                    metrics.mb_per_second,
                    "🧩".yellow(),
                    metrics.chunks_per_second as u32,
                    "🧠".purple(),
                    get_current_memory_usage_mb(),
                );
                
                last_update = Instant::now();
            }
            
            thread::sleep(Duration::from_millis(100));
        }
    }
}
```

This comprehensive approach ensures Digstore Min can efficiently handle both large files and large numbers of small files while maintaining excellent performance characteristics.
