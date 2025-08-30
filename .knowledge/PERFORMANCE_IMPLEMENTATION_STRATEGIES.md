# Performance Implementation Strategies

## Overview

This document provides concrete implementation strategies for achieving the performance requirements for both large files and large numbers of small files in Digstore Min.

## Strategy 1: Streaming Architecture for Large Files

### Core Principle
Never load entire files into memory. Process everything as streams with bounded buffers.

### Implementation Approach
```rust
use tokio::io::{AsyncRead, AsyncBufRead, BufReader};
use futures::stream::{Stream, StreamExt};

pub struct StreamingChunker {
    min_chunk_size: usize,
    max_chunk_size: usize,
    target_chunk_size: usize,
    rolling_hash: RollingHash,
}

impl StreamingChunker {
    pub async fn chunk_stream<R>(
        &mut self,
        reader: R,
        progress: Option<ProgressBar>,
    ) -> impl Stream<Item = Result<Chunk>>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::with_capacity(64 * 1024, reader);
        let mut current_chunk = Vec::new();
        let mut offset = 0u64;
        
        stream! {
            let mut buffer = [0u8; 8192];
            
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => {
                        // End of stream - emit final chunk
                        if !current_chunk.is_empty() {
                            yield Ok(self.finalize_chunk(current_chunk, offset));
                        }
                        break;
                    }
                    Ok(n) => {
                        for &byte in &buffer[..n] {
                            current_chunk.push(byte);
                            
                            // Check for chunk boundary
                            if self.should_break_chunk(&current_chunk) {
                                let chunk = self.finalize_chunk(current_chunk, offset);
                                offset += chunk.size as u64;
                                current_chunk = Vec::new();
                                
                                if let Some(ref pb) = progress {
                                    pb.inc(chunk.size as u64);
                                }
                                
                                yield Ok(chunk);
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(e.into());
                        break;
                    }
                }
            }
        }
    }
    
    fn should_break_chunk(&mut self, chunk: &[u8]) -> bool {
        if chunk.len() < self.min_chunk_size {
            return false;
        }
        
        if chunk.len() >= self.max_chunk_size {
            return true;
        }
        
        // Content-defined boundary detection
        if chunk.len() >= self.target_chunk_size {
            let hash = self.rolling_hash.hash(&chunk[chunk.len()-64..]);
            return (hash & 0xFFF) == 0; // Boundary every ~4KB on average
        }
        
        false
    }
}
```

## Strategy 2: Parallel Batch Processing for Small Files

### Core Principle
Group small files into batches and process them in parallel across multiple threads.

### Implementation Approach
```rust
use rayon::prelude::*;
use crossbeam::channel::{bounded, Receiver, Sender};

pub struct ParallelBatchProcessor {
    batch_size: usize,
    worker_count: usize,
    chunk_cache: Arc<DashMap<Hash, ChunkInfo>>,
}

impl ParallelBatchProcessor {
    pub fn process_small_files(
        &self,
        files: Vec<PathBuf>,
        progress: &ProgressBar,
    ) -> Result<Vec<FileEntry>> {
        // Split into batches
        let batches: Vec<_> = files
            .chunks(self.batch_size)
            .map(|batch| batch.to_vec())
            .collect();
        
        progress.set_length(files.len() as u64);
        
        // Process batches in parallel
        let results: Result<Vec<_>> = batches
            .par_iter()
            .map(|batch| {
                let batch_result = self.process_batch(batch)?;
                progress.inc(batch.len() as u64);
                Ok(batch_result)
            })
            .collect();
        
        // Flatten results
        Ok(results?.into_iter().flatten().collect())
    }
    
    fn process_batch(&self, files: &[PathBuf]) -> Result<Vec<FileEntry>> {
        // Process files in batch sequentially (within thread)
        // This reduces thread contention while maintaining parallelism
        files.iter()
            .map(|path| self.process_single_file_optimized(path))
            .collect()
    }
    
    fn process_single_file_optimized(&self, path: &Path) -> Result<FileEntry> {
        let data = std::fs::read(path)?;
        
        // Optimize for small files
        if data.len() <= 4096 {
            // Very small files: single chunk, no CDC overhead
            let hash = sha256(&data);
            return Ok(FileEntry {
                path: path.to_path_buf(),
                hash,
                size: data.len() as u64,
                chunks: vec![ChunkRef { hash, offset: 0, size: data.len() as u32 }],
            });
        }
        
        // Larger small files: use efficient chunking
        let chunks = self.chunk_small_file(&data)?;
        let file_hash = self.compute_file_hash(&chunks);
        
        Ok(FileEntry {
            path: path.to_path_buf(),
            hash: file_hash,
            size: data.len() as u64,
            chunks,
        })
    }
}
```

## Strategy 3: Adaptive Processing Pipeline

### Core Principle
Automatically detect workload characteristics and adapt processing strategy accordingly.

### Implementation Approach
```rust
pub struct AdaptiveProcessor {
    small_file_processor: ParallelBatchProcessor,
    large_file_processor: StreamingProcessor,
    workload_analyzer: WorkloadAnalyzer,
}

impl AdaptiveProcessor {
    pub async fn process_files_adaptive(
        &self,
        files: Vec<PathBuf>,
        progress: &MultiStageProgress,
    ) -> Result<Vec<FileEntry>> {
        // Analyze workload first
        let analysis = self.workload_analyzer.analyze_files(&files).await?;
        
        progress.set_stage_weights(&[
            ("Analyzing", 5.0),
            ("Processing", 85.0),
            ("Finalizing", 10.0),
        ]);
        
        progress.update_stage(0, 100, 100); // Analysis complete
        
        match analysis.workload_type {
            WorkloadType::ManySmallFiles => {
                progress.set_message("Optimizing for many small files...");
                self.process_small_files_optimized(&files, progress).await
            }
            WorkloadType::FewLargeFiles => {
                progress.set_message("Optimizing for large files...");
                self.process_large_files_optimized(&files, progress).await
            }
            WorkloadType::Mixed => {
                progress.set_message("Using hybrid processing...");
                self.process_mixed_workload(&files, &analysis, progress).await
            }
        }
    }
    
    async fn process_mixed_workload(
        &self,
        files: &[PathBuf],
        analysis: &WorkloadAnalysis,
        progress: &MultiStageProgress,
    ) -> Result<Vec<FileEntry>> {
        let mut results = Vec::with_capacity(files.len());
        
        // Process small files in parallel batches
        if !analysis.small_files.is_empty() {
            let small_results = self.small_file_processor
                .process_files(&analysis.small_files, progress.get_sub_progress(0.6))
                .await?;
            results.extend(small_results);
        }
        
        // Process large files with streaming
        if !analysis.large_files.is_empty() {
            let large_results = self.large_file_processor
                .process_files(&analysis.large_files, progress.get_sub_progress(0.4))
                .await?;
            results.extend(large_results);
        }
        
        Ok(results)
    }
}

pub struct WorkloadAnalyzer;

impl WorkloadAnalyzer {
    pub async fn analyze_files(&self, files: &[PathBuf]) -> Result<WorkloadAnalysis> {
        // Quick metadata scan to categorize files
        let mut small_files = Vec::new();
        let mut large_files = Vec::new();
        let mut total_size = 0u64;
        
        for path in files {
            if let Ok(metadata) = tokio::fs::metadata(path).await {
                let size = metadata.len();
                total_size += size;
                
                if size < 64 * 1024 {
                    small_files.push(path.clone());
                } else {
                    large_files.push(path.clone());
                }
            }
        }
        
        let workload_type = if large_files.is_empty() {
            WorkloadType::ManySmallFiles
        } else if small_files.len() < files.len() / 10 {
            WorkloadType::FewLargeFiles
        } else {
            WorkloadType::Mixed
        };
        
        Ok(WorkloadAnalysis {
            workload_type,
            small_files,
            large_files,
            total_size,
            estimated_processing_time: self.estimate_processing_time(files.len(), total_size),
        })
    }
}
```

## Strategy 4: Memory-Efficient Data Structures

### Core Principle
Use compact data structures and memory pools to minimize allocation overhead.

### Implementation Approach
```rust
use object_pool::{Pool, Reusable};
use compact_str::CompactString;

pub struct MemoryEfficientStorage {
    // Buffer pools for different sizes
    small_buffer_pool: Pool<Vec<u8>>,
    medium_buffer_pool: Pool<Vec<u8>>,
    large_buffer_pool: Pool<Vec<u8>>,
    
    // String interning for path deduplication
    path_interner: StringInterner,
    
    // Compact file entries
    file_entries: Vec<CompactFileEntry>,
}

#[derive(Clone)]
pub struct CompactFileEntry {
    path_id: u32,           // Index into string interner
    hash: Hash,             // 32 bytes
    size: u32,              // 4 bytes (supports files up to 4GB)
    chunk_start: u32,       // 4 bytes (index into chunk array)
    chunk_count: u16,       // 2 bytes (up to 65k chunks per file)
    flags: u16,             // 2 bytes (various flags)
}
// Total: 48 bytes per file (vs 80+ bytes for full PathBuf)

impl MemoryEfficientStorage {
    pub fn add_file_efficient(&mut self, path: &Path, chunks: Vec<Chunk>) -> Result<()> {
        // Intern path string to avoid duplication
        let path_id = self.path_interner.intern(path.to_string_lossy());
        
        // Compute file hash from chunks
        let file_hash = self.compute_file_hash_from_chunks(&chunks);
        
        // Add chunks to global chunk array
        let chunk_start = self.chunks.len() as u32;
        self.chunks.extend(chunks.into_iter().map(CompactChunk::from));
        
        // Create compact file entry
        let file_entry = CompactFileEntry {
            path_id,
            hash: file_hash,
            size: chunks.iter().map(|c| c.size as u64).sum::<u64>() as u32,
            chunk_start,
            chunk_count: chunks.len() as u16,
            flags: 0,
        };
        
        self.file_entries.push(file_entry);
        Ok(())
    }
    
    pub fn get_buffer(&self, size: usize) -> Reusable<Vec<u8>> {
        let pool = match size {
            0..=4096 => &self.small_buffer_pool,
            4097..=65536 => &self.medium_buffer_pool,
            _ => &self.large_buffer_pool,
        };
        
        let mut buffer = pool.try_pull().unwrap_or_else(|| {
            pool.attach(Vec::with_capacity(size))
        });
        
        buffer.clear();
        buffer.resize(size, 0);
        buffer
    }
}
```

## Strategy 5: Optimized Staging and Persistence

### Core Principle
Minimize disk I/O by batching operations and using efficient serialization.

### Implementation Approach
```rust
use std::collections::VecDeque;

pub struct OptimizedStagingManager {
    // In-memory staging with efficient data structures
    staged_files: IndexMap<InternedPath, StagedFile>,
    
    // Batch operations for disk persistence
    pending_writes: VecDeque<StagingBatch>,
    batch_size: usize,
    write_threshold: Duration,
    last_write: Instant,
    
    // Background persistence thread
    persistence_handle: Option<thread::JoinHandle<()>>,
    persistence_channel: Sender<StagingBatch>,
}

#[derive(Serialize, Deserialize)]
pub struct StagingBatch {
    batch_id: uuid::Uuid,
    files: Vec<StagedFile>,
    timestamp: i64,
    checksum: Hash,
}

impl OptimizedStagingManager {
    pub fn stage_files_bulk(&mut self, files: Vec<(PathBuf, StagedFile)>) -> Result<()> {
        // Add to in-memory staging
        for (path, staged_file) in files {
            let interned_path = self.path_interner.intern(path);
            self.staged_files.insert(interned_path, staged_file);
        }
        
        // Check if we should persist
        if self.should_persist() {
            self.create_persistence_batch()?;
        }
        
        Ok(())
    }
    
    fn should_persist(&self) -> bool {
        self.staged_files.len() >= self.batch_size
            || self.last_write.elapsed() >= self.write_threshold
    }
    
    fn create_persistence_batch(&mut self) -> Result<()> {
        if self.staged_files.is_empty() {
            return Ok(());
        }
        
        // Create batch for background persistence
        let batch = StagingBatch {
            batch_id: uuid::Uuid::new_v4(),
            files: self.staged_files.values().cloned().collect(),
            timestamp: chrono::Utc::now().timestamp(),
            checksum: self.compute_batch_checksum(),
        };
        
        // Send to background thread for persistence
        self.persistence_channel.send(batch)?;
        self.last_write = Instant::now();
        
        Ok(())
    }
    
    pub fn commit_all(&mut self) -> Result<Vec<StagedFile>> {
        // Ensure all pending writes are completed
        self.flush_pending_writes()?;
        
        // Return all staged files
        let files = self.staged_files.values().cloned().collect();
        self.staged_files.clear();
        
        // Clear staging on disk
        self.clear_staging_files()?;
        
        Ok(files)
    }
}
```

## Strategy 6: Lock-Free Concurrent Processing

### Core Principle
Use lock-free data structures and atomic operations to minimize contention.

### Implementation Approach
```rust
use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};

pub struct LockFreePipeline {
    // Lock-free queues for work distribution
    file_queue: SegQueue<FileWork>,
    chunk_queue: SegQueue<ChunkWork>,
    result_queue: SegQueue<ProcessedFile>,
    
    // Atomic counters for progress tracking
    files_queued: AtomicUsize,
    files_processed: AtomicUsize,
    bytes_processed: AtomicU64,
    
    // Worker thread management
    active_workers: AtomicUsize,
    shutdown_signal: AtomicBool,
}

impl LockFreePipeline {
    pub fn process_files_lockfree(&self, files: Vec<PathBuf>) -> Result<Vec<FileEntry>> {
        // Initialize counters
        self.files_queued.store(files.len(), Ordering::SeqCst);
        self.files_processed.store(0, Ordering::SeqCst);
        self.bytes_processed.store(0, Ordering::SeqCst);
        
        // Queue all files
        for file in files {
            self.file_queue.push(FileWork::Process(file));
        }
        
        // Start worker threads
        let workers = self.start_workers();
        
        // Monitor progress
        let progress_handle = self.start_progress_monitor();
        
        // Wait for completion
        while self.files_processed.load(Ordering::Relaxed) < self.files_queued.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
        
        // Signal shutdown and wait for workers
        self.shutdown_signal.store(true, Ordering::SeqCst);
        for worker in workers {
            worker.join().unwrap();
        }
        progress_handle.join().unwrap();
        
        // Collect results
        let mut results = Vec::new();
        while let Some(processed) = self.result_queue.pop() {
            results.push(processed.file_entry);
        }
        
        Ok(results)
    }
    
    fn start_workers(&self) -> Vec<thread::JoinHandle<()>> {
        (0..num_cpus::get())
            .map(|worker_id| {
                let file_queue = &self.file_queue;
                let result_queue = &self.result_queue;
                let shutdown = &self.shutdown_signal;
                let processed_counter = &self.files_processed;
                let bytes_counter = &self.bytes_processed;
                
                thread::spawn(move || {
                    while !shutdown.load(Ordering::Relaxed) {
                        if let Some(work) = file_queue.pop() {
                            match work {
                                FileWork::Process(path) => {
                                    match process_file_fast(&path) {
                                        Ok(file_entry) => {
                                            bytes_counter.fetch_add(file_entry.size, Ordering::Relaxed);
                                            result_queue.push(ProcessedFile { file_entry });
                                            processed_counter.fetch_add(1, Ordering::Relaxed);
                                        }
                                        Err(e) => {
                                            eprintln!("Worker {}: Error processing {}: {}", 
                                                     worker_id, path.display(), e);
                                        }
                                    }
                                }
                            }
                        } else {
                            // No work available, sleep briefly
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                })
            })
            .collect()
    }
}
```

## Strategy 7: Intelligent Caching System

### Core Principle
Cache frequently accessed data with smart eviction policies.

### Implementation Approach
```rust
use lru::LruCache;
use std::num::NonZeroUsize;

pub struct IntelligentCache {
    // Multi-level cache system
    hot_cache: Mutex<LruCache<Hash, Arc<Vec<u8>>>>,     // Recently used chunks
    warm_cache: Mutex<LruCache<Hash, Arc<Vec<u8>>>>,    // Occasionally used chunks
    cold_storage: DashMap<Hash, CacheMetadata>,         // Metadata only
    
    // Cache statistics
    hit_count: AtomicU64,
    miss_count: AtomicU64,
    eviction_count: AtomicU64,
    
    // Configuration
    hot_cache_size: usize,
    warm_cache_size: usize,
    total_memory_limit: usize,
}

impl IntelligentCache {
    pub fn get_chunk(&self, hash: &Hash) -> Option<Arc<Vec<u8>>> {
        // Try hot cache first
        if let Some(chunk) = self.hot_cache.lock().get(hash) {
            self.hit_count.fetch_add(1, Ordering::Relaxed);
            return Some(chunk.clone());
        }
        
        // Try warm cache
        if let Some(chunk) = self.warm_cache.lock().get(hash) {
            // Promote to hot cache
            self.promote_to_hot_cache(hash.clone(), chunk.clone());
            self.hit_count.fetch_add(1, Ordering::Relaxed);
            return Some(chunk);
        }
        
        // Cache miss
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        None
    }
    
    pub fn put_chunk(&self, hash: Hash, data: Arc<Vec<u8>>) -> Result<()> {
        let data_size = data.len();
        
        // Check memory pressure
        if self.get_total_memory_usage() + data_size > self.total_memory_limit {
            self.evict_cold_data(data_size)?;
        }
        
        // Add to hot cache
        let evicted = self.hot_cache.lock().put(hash, data);
        
        // If something was evicted, move to warm cache
        if let Some((evicted_hash, evicted_data)) = evicted {
            self.warm_cache.lock().put(evicted_hash, evicted_data);
            self.eviction_count.fetch_add(1, Ordering::Relaxed);
        }
        
        Ok(())
    }
    
    fn evict_cold_data(&self, needed_space: usize) -> Result<()> {
        let mut freed_space = 0;
        
        // Evict from warm cache first
        while freed_space < needed_space {
            if let Some((_, data)) = self.warm_cache.lock().pop_lru() {
                freed_space += data.len();
            } else {
                break;
            }
        }
        
        // If still not enough space, evict from hot cache
        while freed_space < needed_space {
            if let Some((_, data)) = self.hot_cache.lock().pop_lru() {
                freed_space += data.len();
            } else {
                break;
            }
        }
        
        Ok(())
    }
}
```

## Strategy 8: Performance Monitoring and Auto-Tuning

### Core Principle
Continuously monitor performance and automatically adjust parameters for optimal throughput.

### Implementation Approach
```rust
pub struct PerformanceAutoTuner {
    current_config: Mutex<ProcessingConfig>,
    performance_history: Mutex<VecDeque<PerformanceSample>>,
    tuning_interval: Duration,
    last_tuning: Mutex<Instant>,
}

#[derive(Clone)]
pub struct ProcessingConfig {
    batch_size: usize,
    worker_count: usize,
    buffer_size: usize,
    chunk_cache_size: usize,
}

impl PerformanceAutoTuner {
    pub fn maybe_tune_performance(&self, current_metrics: &PerformanceMetrics) -> Result<()> {
        let mut last_tuning = self.last_tuning.lock().unwrap();
        
        if last_tuning.elapsed() < self.tuning_interval {
            return Ok(());
        }
        
        // Record current performance
        let sample = PerformanceSample {
            config: self.current_config.lock().unwrap().clone(),
            files_per_second: current_metrics.files_per_second,
            mb_per_second: current_metrics.mb_per_second,
            memory_usage: current_metrics.peak_memory_mb,
            timestamp: Instant::now(),
        };
        
        self.performance_history.lock().unwrap().push_back(sample);
        
        // Analyze and adjust
        if let Some(new_config) = self.analyze_and_optimize()? {
            *self.current_config.lock().unwrap() = new_config;
            println!("🔧 Auto-tuned processing parameters for better performance");
        }
        
        *last_tuning = Instant::now();
        Ok(())
    }
    
    fn analyze_and_optimize(&self) -> Result<Option<ProcessingConfig>> {
        let history = self.performance_history.lock().unwrap();
        let current_config = self.current_config.lock().unwrap().clone();
        
        if history.len() < 3 {
            return Ok(None); // Need more data
        }
        
        // Find best performing configuration
        let best_sample = history.iter()
            .max_by(|a, b| a.files_per_second.partial_cmp(&b.files_per_second).unwrap())
            .unwrap();
        
        let latest_sample = history.back().unwrap();
        
        // If current performance is significantly worse than best, adjust
        if latest_sample.files_per_second < best_sample.files_per_second * 0.9 {
            let mut new_config = best_sample.config.clone();
            
            // Try incremental improvements
            if latest_sample.memory_usage < best_sample.memory_usage * 0.8 {
                // We have memory headroom, increase batch size
                new_config.batch_size = (new_config.batch_size * 110 / 100).min(1000);
            }
            
            return Ok(Some(new_config));
        }
        
        Ok(None)
    }
}
```

## Implementation Priority

### Phase 1: Foundation (Current Status ✅)
- ✅ Basic chunking and hashing working
- ✅ Simple file operations implemented
- ✅ CLI interface functional

### Phase 2: Small File Optimization (Next Priority)
1. **Implement batch processing** (highest impact)
2. **Add parallel file processing** (CPU utilization)
3. **Optimize staging area** (reduce I/O overhead)
4. **Add performance monitoring** (measure improvements)

### Phase 3: Large File Optimization
1. **Implement streaming chunking** (memory efficiency)
2. **Add memory-mapped file support** (large file performance)
3. **Create backpressure handling** (stability)
4. **Add progress feedback** (user experience)

### Phase 4: Advanced Optimizations
1. **Implement adaptive processing** (automatic optimization)
2. **Add intelligent caching** (performance consistency)
3. **Create auto-tuning system** (continuous improvement)
4. **Add comprehensive benchmarks** (validation)

## Success Metrics

### Quantitative Metrics
- ✅ 20,000 small files commit: <60 seconds
- ✅ Large file (5GB) processing: >500 MB/s
- ✅ Memory usage: <200MB for large files, <500MB for 20k small files
- ✅ CPU utilization: >80% during processing
- ✅ I/O efficiency: >80% of storage bandwidth

### Qualitative Metrics
- ✅ Responsive progress feedback
- ✅ Predictable memory usage
- ✅ Graceful handling of resource constraints
- ✅ Consistent performance across platforms
- ✅ Scalability with hardware improvements

## Testing Strategy

### 1. Performance Regression Tests
```rust
#[test]
fn performance_regression_test() {
    // Baseline performance requirements
    let requirements = PerformanceBaseline {
        small_files_per_second: 300.0,
        large_file_mb_per_second: 500.0,
        max_memory_mb: 500,
        max_commit_time_20k_files: Duration::from_secs(60),
    };
    
    // Run standard test suite
    let metrics = run_performance_test_suite()?;
    
    // Validate against requirements
    assert!(metrics.small_files_per_second >= requirements.small_files_per_second,
            "Small file processing too slow: {} < {}", 
            metrics.small_files_per_second, requirements.small_files_per_second);
            
    assert!(metrics.large_file_mb_per_second >= requirements.large_file_mb_per_second,
            "Large file processing too slow: {} < {}",
            metrics.large_file_mb_per_second, requirements.large_file_mb_per_second);
            
    assert!(metrics.peak_memory_mb <= requirements.max_memory_mb,
            "Memory usage too high: {} > {}",
            metrics.peak_memory_mb, requirements.max_memory_mb);
}
```

### 2. Continuous Performance Monitoring
```bash
#!/bin/bash
# performance_ci.sh - Run in CI to detect regressions

echo "Running performance regression tests..."

# Test small files performance
cargo run --release -- init --name "perf-test"
time cargo run --release -- add tests/fixtures/small_files/
time cargo run --release -- commit -m "Small files test"

# Test large file performance  
time cargo run --release -- add tests/fixtures/large_file.bin
time cargo run --release -- commit -m "Large file test"

# Run benchmarks
cargo bench --bench chunking -- --save-baseline main

echo "Performance tests completed successfully"
```

This comprehensive performance optimization framework ensures Digstore Min can efficiently handle both large files and large numbers of small files while maintaining excellent user experience and system responsiveness.
