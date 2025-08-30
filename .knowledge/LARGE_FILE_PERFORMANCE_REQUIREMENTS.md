# Large File Performance Requirements

## Overview

Digstore Min must efficiently handle both large files (multi-GB) and large numbers of small files (20,000+) with excellent performance characteristics. This document specifies the requirements and implementation strategies for achieving these goals.

## Large File Support Requirements

### 1. Memory Efficiency
- **File Pointers Only**: Never load entire files into memory, only store file pointers and metadata
- **Streaming I/O**: Process files as streams with bounded buffers (64KB-1MB)
- **Chunk References**: Store only chunk metadata (hash, offset, size), not chunk data
- **On-demand Reconstruction**: Read chunks from original files only when needed
- **Memory-mapped files**: Use mmap for efficient large file access when appropriate
- **Constant memory usage**: Memory consumption should not scale with file size
- **Backpressure handling**: Prevent memory exhaustion during high-throughput operations

### 2. Performance Targets
- **Chunking speed**: >500 MB/s for large files (>1GB)
- **Hashing speed**: >1 GB/s SHA-256 throughput
- **I/O throughput**: >80% of underlying storage bandwidth
- **Memory overhead**: <100MB regardless of file size
- **Progress feedback**: Real-time progress for operations >5 seconds

### 3. Large File Scenarios
- **Multi-GB files**: Video files, database dumps, archives
- **Very large files**: >10GB files (dataset files, VM images)
- **Streaming data**: Continuous data streams without known end
- **Network sources**: HTTP streams, S3 objects, remote files

## Small File Performance Requirements

### 1. Batch Processing Efficiency
- **20,000 small files**: Complete add/commit cycle in <60 seconds
- **Parallel processing**: Utilize all CPU cores for file processing
- **Batch I/O**: Group small file operations to reduce syscall overhead
- **Index optimization**: Efficient in-memory indexing for file lookups
- **Staging optimization**: Fast staging area management

### 2. Performance Targets for Small Files
- **File scanning**: >1,000 files/second during directory traversal
- **Chunking small files**: >10,000 files/second (files <64KB)
- **Hash computation**: >2,000 files/second for typical source files
- **Staging operations**: >5,000 files/second for add operations
- **Commit creation**: Complete 20,000 file commit in <30 seconds

### 3. Small File Scenarios
- **Source code repositories**: Thousands of .rs, .js, .py files
- **Documentation sites**: Many small markdown, HTML, CSS files
- **Configuration management**: Hundreds of config files
- **Asset directories**: Images, icons, small media files

## Implementation Strategies

### 1. Streaming Architecture
```rust
pub struct StreamingProcessor {
    buffer_size: usize,
    chunk_size: usize,
    parallel_workers: usize,
}

impl StreamingProcessor {
    // Never load entire file into memory
    pub async fn process_large_file<R, F>(
        &self,
        reader: R,
        processor: F,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin,
        F: Fn(&[u8]) -> Result<()>,
    {
        let mut reader = BufReader::with_capacity(self.buffer_size, reader);
        let mut buffer = vec![0u8; self.chunk_size];
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 { break; }
            
            processor(&buffer[..n])?;
        }
        
        Ok(())
    }
}
```

### 2. Parallel Small File Processing
```rust
use rayon::prelude::*;
use dashmap::DashMap;

pub struct BatchProcessor {
    chunk_cache: Arc<DashMap<Hash, Vec<u8>>>,
    file_cache: Arc<DashMap<PathBuf, FileEntry>>,
}

impl BatchProcessor {
    pub fn process_small_files_parallel(
        &self,
        files: Vec<PathBuf>,
        progress: Option<ProgressBar>,
    ) -> Result<Vec<FileEntry>> {
        files
            .par_chunks(100) // Process in batches of 100
            .map(|batch| {
                batch.iter()
                    .map(|path| self.process_single_file(path))
                    .collect::<Result<Vec<_>>>()
            })
            .reduce(
                || Ok(Vec::new()),
                |acc, batch| {
                    let mut acc = acc?;
                    acc.extend(batch?);
                    if let Some(ref pb) = progress {
                        pb.inc(100);
                    }
                    Ok(acc)
                }
            )
    }
}
```

### 3. Memory-Mapped Large File Access
```rust
use memmap2::{Mmap, MmapOptions};

pub struct LargeFileHandler {
    max_memory_map_size: u64,
}

impl LargeFileHandler {
    pub fn process_large_file(&self, path: &Path) -> Result<Vec<Chunk>> {
        let file = File::open(path)?;
        let file_size = file.metadata()?.len();
        
        if file_size > self.max_memory_map_size {
            // Use streaming for very large files
            self.stream_process_file(file)
        } else {
            // Use memory mapping for moderately large files
            let mmap = unsafe { MmapOptions::new().map(&file)? };
            self.chunk_mapped_data(&mmap)
        }
    }
    
    fn chunk_mapped_data(&self, data: &[u8]) -> Result<Vec<Chunk>> {
        // Process memory-mapped data in chunks
        let mut chunks = Vec::new();
        let chunk_size = 1024 * 1024; // 1MB chunks
        
        for (i, chunk_data) in data.chunks(chunk_size).enumerate() {
            let chunk = Chunk {
                offset: i * chunk_size,
                size: chunk_data.len(),
                hash: sha256(chunk_data),
                data: chunk_data.to_vec(),
            };
            chunks.push(chunk);
        }
        
        Ok(chunks)
    }
}
```

### 4. Optimized Staging Area
```rust
use indexmap::IndexMap;

pub struct OptimizedStagingArea {
    // Use IndexMap for ordered iteration with O(1) lookups
    staged_files: IndexMap<PathBuf, StagedFile>,
    // Batch operations for disk persistence
    dirty: bool,
    batch_size: usize,
}

impl OptimizedStagingArea {
    pub fn add_files_batch(&mut self, files: Vec<(PathBuf, StagedFile)>) -> Result<()> {
        // Add multiple files in a single operation
        for (path, file) in files {
            self.staged_files.insert(path, file);
        }
        
        self.dirty = true;
        
        // Batch persistence - only write to disk when batch is full
        if self.staged_files.len() % self.batch_size == 0 {
            self.persist_to_disk()?;
        }
        
        Ok(())
    }
    
    pub fn commit_all(&mut self) -> Result<Vec<FileEntry>> {
        if self.dirty {
            self.persist_to_disk()?;
        }
        
        let files: Vec<_> = self.staged_files.values().cloned().collect();
        self.staged_files.clear();
        self.dirty = true;
        self.persist_to_disk()?; // Clear staging on disk
        
        Ok(files)
    }
}
```

### 5. Efficient Layer Writing
```rust
pub struct OptimizedLayerWriter {
    buffer_size: usize,
    compression_threshold: usize,
}

impl OptimizedLayerWriter {
    pub async fn write_layer_streaming(
        &self,
        layer: &Layer,
        output: &mut (dyn AsyncWrite + Unpin),
    ) -> Result<Hash> {
        let mut writer = BufWriter::with_capacity(self.buffer_size, output);
        let mut hasher = Sha256::new();
        
        // Write header
        let header_bytes = layer.header.to_bytes();
        writer.write_all(&header_bytes).await?;
        hasher.update(&header_bytes);
        
        // Stream file data without loading all into memory
        for file_entry in &layer.files {
            let file_bytes = self.serialize_file_entry(file_entry)?;
            writer.write_all(&file_bytes).await?;
            hasher.update(&file_bytes);
        }
        
        // Stream chunk data
        for chunk in &layer.chunks {
            let chunk_data = if chunk.data.len() > self.compression_threshold {
                compress_chunk(&chunk.data)?
            } else {
                chunk.data.clone()
            };
            
            writer.write_all(&chunk_data).await?;
            hasher.update(&chunk_data);
        }
        
        writer.flush().await?;
        Ok(Hash::from_bytes(hasher.finalize().into()))
    }
}
```

## Performance Monitoring

### 1. Metrics Collection
```rust
pub struct PerformanceMetrics {
    pub files_processed: u64,
    pub bytes_processed: u64,
    pub chunks_created: u64,
    pub deduplication_ratio: f64,
    pub processing_time: Duration,
    pub memory_peak: u64,
}

impl PerformanceMetrics {
    pub fn throughput_mb_per_sec(&self) -> f64 {
        let bytes_per_sec = self.bytes_processed as f64 / self.processing_time.as_secs_f64();
        bytes_per_sec / (1024.0 * 1024.0)
    }
    
    pub fn files_per_sec(&self) -> f64 {
        self.files_processed as f64 / self.processing_time.as_secs_f64()
    }
}
```

### 2. Progress Reporting for Large Operations
```rust
pub struct LargeOperationProgress {
    multi_progress: MultiProgress,
    file_progress: ProgressBar,
    byte_progress: ProgressBar,
    chunk_progress: ProgressBar,
}

impl LargeOperationProgress {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        let multi = MultiProgress::new();
        
        let file_progress = multi.add(ProgressBar::new(total_files));
        file_progress.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} Files: [{bar:20.cyan/blue}] {pos}/{len} ({per_sec}) {msg}")
                .unwrap()
        );
        
        let byte_progress = multi.add(ProgressBar::new(total_bytes));
        byte_progress.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.blue} Bytes: [{bar:20.green/yellow}] {bytes}/{total_bytes} ({bytes_per_sec}) ETA: {eta}")
                .unwrap()
        );
        
        Self { multi_progress: multi, file_progress, byte_progress, chunk_progress }
    }
}
```

## Configuration for Performance

### 1. Adaptive Chunk Sizes
```toml
[chunking]
# Small files (<1MB): larger chunks for efficiency
small_file_chunk_size = "256KB"
small_file_threshold = "1MB"

# Medium files (1MB-100MB): standard chunking
medium_file_chunk_size = "1MB"
medium_file_threshold = "100MB"

# Large files (>100MB): smaller chunks for parallelism
large_file_chunk_size = "512KB"

# Very large files (>1GB): optimize for streaming
huge_file_chunk_size = "2MB"
huge_file_threshold = "1GB"
```

### 2. Performance Tuning
```toml
[performance]
# Parallel processing
max_worker_threads = 0  # 0 = auto-detect CPU cores
batch_size = 100        # Files per batch for small file processing
buffer_size = "64KB"    # I/O buffer size

# Memory management
max_memory_usage = "2GB"     # Maximum memory for caching
chunk_cache_size = "500MB"   # Chunk deduplication cache
file_cache_size = "100MB"    # File metadata cache

# I/O optimization
async_io = true              # Use async I/O for large operations
memory_map_threshold = "10MB" # Use mmap for files larger than this
compression_threshold = "4KB" # Compress chunks larger than this
```

## Testing Requirements

### 1. Large File Test Cases
```rust
#[test]
fn test_10gb_file_processing() {
    // Create 10GB sparse file for testing
    let large_file = create_sparse_file(10 * 1024 * 1024 * 1024);
    
    let start = Instant::now();
    let chunks = chunk_large_file(&large_file)?;
    let duration = start.elapsed();
    
    // Should process at >500 MB/s
    let throughput = (10 * 1024 * 1024 * 1024) as f64 / duration.as_secs_f64();
    assert!(throughput > 500.0 * 1024.0 * 1024.0);
    
    // Memory usage should be constant
    let memory_usage = get_memory_usage();
    assert!(memory_usage < 200 * 1024 * 1024); // <200MB
}

#[test]
fn test_20000_small_files() {
    // Create 20,000 small files (1-10KB each)
    let temp_dir = create_test_files(20000, 1024..10240);
    
    let start = Instant::now();
    
    // Add all files
    let mut store = Store::init(&temp_dir)?;
    store.add_directory(&temp_dir, true)?;
    
    // Commit all files
    let commit_id = store.commit("Add 20,000 files")?;
    
    let duration = start.elapsed();
    
    // Should complete in <60 seconds
    assert!(duration.as_secs() < 60);
    
    // Should process >300 files/second
    let files_per_sec = 20000.0 / duration.as_secs_f64();
    assert!(files_per_sec > 300.0);
}
```

### 2. Performance Benchmarks
```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};

fn bench_large_file_chunking(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_file_chunking");
    
    // Test different file sizes
    for size in [1_000_000, 10_000_000, 100_000_000, 1_000_000_000] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("chunk", size),
            &size,
            |b, &size| {
                let data = generate_test_data(size);
                b.iter(|| {
                    chunk_data(&data)
                });
            },
        );
    }
    
    group.finish();
}

fn bench_many_small_files(c: &mut Criterion) {
    let mut group = c.benchmark_group("many_small_files");
    
    // Test different file counts
    for count in [1000, 5000, 10000, 20000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("process", count),
            &count,
            |b, &count| {
                let files = generate_small_test_files(count);
                b.iter(|| {
                    process_files_batch(&files)
                });
            },
        );
    }
    
    group.finish();
}
```

## Implementation Requirements

### 1. Streaming Chunking Engine
```rust
pub struct StreamingChunkingEngine {
    config: ChunkConfig,
    rolling_hash: RollingHash,
    buffer: CircularBuffer,
}

impl StreamingChunkingEngine {
    pub async fn chunk_stream<R>(
        &mut self,
        reader: R,
        chunk_callback: impl Fn(Chunk) -> Result<()>,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::new(reader);
        let mut buffer = vec![0u8; 64 * 1024];
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 { break; }
            
            for chunk in self.process_buffer(&buffer[..n])? {
                chunk_callback(chunk)?;
            }
        }
        
        Ok(())
    }
}
```

### 2. Parallel File Processing Pipeline
```rust
use crossbeam::channel::{bounded, Receiver, Sender};
use std::thread;

pub struct ParallelFileProcessor {
    worker_count: usize,
    batch_size: usize,
}

impl ParallelFileProcessor {
    pub fn process_files(
        &self,
        files: Vec<PathBuf>,
        progress: ProgressBar,
    ) -> Result<Vec<FileEntry>> {
        let (file_sender, file_receiver): (Sender<PathBuf>, Receiver<PathBuf>) = bounded(1000);
        let (result_sender, result_receiver): (Sender<FileEntry>, Receiver<FileEntry>) = bounded(1000);
        
        // Spawn worker threads
        let workers: Vec<_> = (0..self.worker_count)
            .map(|_| {
                let file_rx = file_receiver.clone();
                let result_tx = result_sender.clone();
                thread::spawn(move || {
                    while let Ok(file_path) = file_rx.recv() {
                        match process_single_file(&file_path) {
                            Ok(file_entry) => {
                                result_tx.send(file_entry).unwrap();
                            }
                            Err(e) => {
                                eprintln!("Error processing {}: {}", file_path.display(), e);
                            }
                        }
                    }
                })
            })
            .collect();
        
        // Send files to workers
        thread::spawn(move || {
            for file in files {
                file_sender.send(file).unwrap();
            }
        });
        
        // Collect results
        let mut results = Vec::new();
        for _ in 0..files.len() {
            results.push(result_receiver.recv()?);
            progress.inc(1);
        }
        
        // Wait for workers to finish
        for worker in workers {
            worker.join().unwrap();
        }
        
        Ok(results)
    }
}
```

### 3. Efficient Layer Construction
```rust
pub struct EfficientLayerBuilder {
    file_entries: Vec<FileEntry>,
    chunk_dedup_map: DashMap<Hash, ChunkLocation>,
    merkle_builder: IncrementalMerkleBuilder,
}

impl EfficientLayerBuilder {
    pub fn add_file_batch(&mut self, files: Vec<FileEntry>) -> Result<()> {
        // Process files in parallel
        let processed_files: Vec<_> = files
            .par_iter()
            .map(|file| self.process_file_entry(file))
            .collect::<Result<Vec<_>>>()?;
        
        // Add to layer in single operation
        self.file_entries.extend(processed_files);
        
        Ok(())
    }
    
    pub fn build_layer_incremental(&mut self) -> Result<Layer> {
        // Build merkle tree incrementally as files are added
        let merkle_root = self.merkle_builder.finalize()?;
        
        // Create layer with deduplicated chunks
        let unique_chunks: Vec<_> = self.chunk_dedup_map
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        
        Layer::new(
            LayerType::Full,
            self.file_entries.clone(),
            unique_chunks,
            merkle_root,
        )
    }
}
```

## Memory Management

### 1. Chunk Cache with LRU Eviction
```rust
use lru::LruCache;

pub struct ChunkCache {
    cache: Mutex<LruCache<Hash, Arc<Vec<u8>>>>,
    max_memory: usize,
    current_memory: AtomicUsize,
}

impl ChunkCache {
    pub fn get_or_load(&self, hash: &Hash) -> Result<Arc<Vec<u8>>> {
        // Try cache first
        if let Some(chunk) = self.cache.lock().get(hash) {
            return Ok(chunk.clone());
        }
        
        // Load from disk
        let chunk_data = Arc::new(self.load_chunk_from_disk(hash)?);
        
        // Add to cache with memory management
        self.add_to_cache_with_eviction(hash.clone(), chunk_data.clone())?;
        
        Ok(chunk_data)
    }
    
    fn add_to_cache_with_eviction(&self, hash: Hash, data: Arc<Vec<u8>>) -> Result<()> {
        let data_size = data.len();
        
        // Evict entries if we would exceed memory limit
        while self.current_memory.load(Ordering::Relaxed) + data_size > self.max_memory {
            if let Some((_, evicted)) = self.cache.lock().pop_lru() {
                self.current_memory.fetch_sub(evicted.len(), Ordering::Relaxed);
            } else {
                break; // Cache is empty
            }
        }
        
        self.cache.lock().put(hash, data);
        self.current_memory.fetch_add(data_size, Ordering::Relaxed);
        
        Ok(())
    }
}
```

### 2. Memory Pool for Buffers
```rust
use object_pool::{Pool, Reusable};

pub struct BufferPool {
    pool: Pool<Vec<u8>>,
    buffer_size: usize,
}

impl BufferPool {
    pub fn new(buffer_size: usize, pool_size: usize) -> Self {
        let pool = Pool::new(pool_size, || {
            Vec::with_capacity(buffer_size)
        });
        
        Self { pool, buffer_size }
    }
    
    pub fn get_buffer(&self) -> Reusable<Vec<u8>> {
        let mut buffer = self.pool.try_pull().unwrap_or_else(|| {
            self.pool.attach(Vec::with_capacity(self.buffer_size))
        });
        
        buffer.clear();
        buffer.resize(self.buffer_size, 0);
        
        buffer
    }
}
```

## Progress Feedback Requirements

### 1. Multi-Stage Progress for Large Operations
```rust
pub struct MultiStageProgress {
    stages: Vec<ProgressStage>,
    current_stage: usize,
    overall_progress: ProgressBar,
}

pub struct ProgressStage {
    name: String,
    weight: f64, // Percentage of total operation
    progress: ProgressBar,
}

impl MultiStageProgress {
    pub fn new(stages: Vec<(&str, f64)>) -> Self {
        let multi = MultiProgress::new();
        let overall = multi.add(ProgressBar::new(100));
        
        let stage_bars: Vec<_> = stages.iter().map(|(name, _)| {
            let pb = multi.add(ProgressBar::new(100));
            pb.set_style(ProgressStyle::default_bar()
                .template(&format!("  {}: [{{bar:20}}] {{percent}}%", name))
                .unwrap());
            ProgressStage {
                name: name.to_string(),
                weight: *weight,
                progress: pb,
            }
        }).collect();
        
        Self {
            stages: stage_bars,
            current_stage: 0,
            overall_progress: overall,
        }
    }
    
    pub fn update_current_stage(&mut self, progress: u64, total: u64) {
        if self.current_stage < self.stages.len() {
            let stage = &self.stages[self.current_stage];
            stage.progress.set_length(total);
            stage.progress.set_position(progress);
            
            // Update overall progress
            let stage_completion = progress as f64 / total as f64;
            let overall_progress: f64 = self.stages[..self.current_stage]
                .iter()
                .map(|s| s.weight)
                .sum::<f64>() + (stage.weight * stage_completion);
                
            self.overall_progress.set_position(overall_progress as u64);
        }
    }
}
```

## Error Recovery and Resilience

### 1. Partial Operation Recovery
```rust
pub struct PartialOperationState {
    completed_files: Vec<PathBuf>,
    failed_files: Vec<(PathBuf, String)>,
    checkpoint_interval: usize,
}

impl PartialOperationState {
    pub fn save_checkpoint(&self, path: &Path) -> Result<()> {
        let checkpoint = serde_json::json!({
            "completed": self.completed_files,
            "failed": self.failed_files,
            "timestamp": chrono::Utc::now().timestamp(),
        });
        
        std::fs::write(path, serde_json::to_string_pretty(&checkpoint)?)?;
        Ok(())
    }
    
    pub fn resume_from_checkpoint(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let checkpoint: serde_json::Value = serde_json::from_str(&content)?;
        
        Ok(Self {
            completed_files: serde_json::from_value(checkpoint["completed"].clone())?,
            failed_files: serde_json::from_value(checkpoint["failed"].clone())?,
            checkpoint_interval: 1000,
        })
    }
}
```

## Performance Validation

### 1. Automated Performance Tests
```rust
#[cfg(test)]
mod performance_tests {
    use super::*;
    
    #[test]
    fn validate_large_file_performance() {
        let requirements = PerformanceRequirements {
            min_chunking_speed_mb_s: 500.0,
            min_hashing_speed_mb_s: 1000.0,
            max_memory_usage_mb: 200,
            max_commit_time_20k_files_sec: 60,
        };
        
        let metrics = run_performance_test()?;
        
        assert!(metrics.chunking_speed_mb_s >= requirements.min_chunking_speed_mb_s);
        assert!(metrics.hashing_speed_mb_s >= requirements.min_hashing_speed_mb_s);
        assert!(metrics.peak_memory_mb <= requirements.max_memory_usage_mb);
    }
}
```

This comprehensive approach ensures Digstore Min can handle both large files and many small files efficiently while providing excellent user feedback through progress indicators.
