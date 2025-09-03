# Performance Requirements for Digstore Min

## Overview

Digstore Min must deliver excellent performance across a wide range of workloads, from handling massive files to processing thousands of small files efficiently. This document specifies the performance requirements and success criteria.

## Core Performance Requirements

### 1. Large File Performance
- **File Size Range**: Support files from 1GB to 100GB+ efficiently
- **Memory Usage**: Constant memory consumption <200MB regardless of file size
- **Chunking Speed**: >500 MB/s for files larger than 1GB
- **Hashing Speed**: >1 GB/s SHA-256 throughput
- **I/O Efficiency**: Achieve >80% of underlying storage bandwidth
- **Streaming**: Never load entire large files into memory

### 2. Small File Performance
- **File Count**: Handle 20,000+ small files efficiently
- **Commit Time**: Complete 20,000 file commit in <60 seconds
- **Processing Speed**: >300 files/second for typical source files (1-50KB)
- **Scanning Speed**: >1,000 files/second during directory traversal
- **Memory Efficiency**: <500MB memory for 20,000 file operations
- **Staging Speed**: >5,000 files/second for add operations

### 3. Mixed Workload Performance
- **Adaptive Processing**: Automatically optimize for file size distribution
- **Parallel Efficiency**: Utilize all available CPU cores effectively
- **Memory Management**: Efficient allocation patterns for mixed workloads
- **Progress Feedback**: Real-time progress without performance impact
- **Scalability**: Performance should scale linearly with hardware

## Detailed Performance Targets

### 1. Chunking Performance
| File Size | Target Speed | Memory Usage | Notes |
|-----------|--------------|--------------|-------|
| 1KB-64KB | >10,000 files/s | <1MB per thread | Skip CDC for tiny files |
| 64KB-1MB | >2,000 files/s | <10MB per thread | Fast CDC with small chunks |
| 1MB-100MB | >500 MB/s | <50MB per thread | Standard CDC |
| 100MB-1GB | >500 MB/s | <100MB per thread | Optimized CDC |
| >1GB | >500 MB/s | <200MB total | Streaming CDC |

### 2. Hashing Performance
| Operation | Target Speed | Memory Usage | Implementation |
|-----------|--------------|--------------|----------------|
| Small files (<64KB) | >2 GB/s | Minimal | Direct hashing |
| Medium files (64KB-10MB) | >1.5 GB/s | <10MB | Buffered hashing |
| Large files (>10MB) | >1 GB/s | <50MB | Streaming hashing |
| Parallel hashing | >4 GB/s | <200MB | Multi-threaded |

### 3. Storage Operations
| Operation | Target Performance | Constraints |
|-----------|-------------------|-------------|
| Layer creation | >1,000 files/s | Memory efficient |
| Layer reading | >2,000 files/s | Lazy loading |
| Staging persistence | >5,000 files/s | Batch operations |
| Index operations | >10,000 lookups/s | In-memory cache |
| Deduplication | >1,000 chunks/s | Lock-free |

## Memory Management Requirements

### 1. Memory Limits
- **Maximum heap usage**: 2GB for any operation
- **Typical usage**: <500MB for normal operations
- **Large file processing**: <200MB regardless of file size
- **Small file batching**: <100MB per 1,000 files
- **Chunk cache**: Configurable, default 500MB

### 2. Memory Allocation Patterns
```rust
// Efficient allocation strategies
pub struct MemoryManager {
    buffer_pool: BufferPool,        // Reuse buffers
    chunk_cache: LruCache<Hash, Arc<Vec<u8>>>,  // Smart caching
    string_interner: StringInterner, // Deduplicate path strings
}

impl MemoryManager {
    pub fn allocate_buffer(&self, size: usize) -> PooledBuffer {
        // Reuse buffers to avoid allocation churn
        self.buffer_pool.get_buffer(size)
    }
    
    pub fn intern_path(&self, path: &Path) -> InternedString {
        // Deduplicate common path prefixes
        self.string_interner.intern(path.to_string_lossy())
    }
}
```

### 3. Garbage Collection Strategy
```rust
pub struct MemoryGC {
    last_gc: Instant,
    gc_interval: Duration,
    memory_pressure_threshold: usize,
}

impl MemoryGC {
    pub fn maybe_collect(&mut self) -> Result<()> {
        let current_memory = get_memory_usage();
        
        if current_memory > self.memory_pressure_threshold 
            || self.last_gc.elapsed() > self.gc_interval {
            
            self.force_collect()?;
            self.last_gc = Instant::now();
        }
        
        Ok(())
    }
    
    fn force_collect(&self) -> Result<()> {
        // Clear caches, drop unused buffers, etc.
        self.buffer_pool.clear_unused();
        self.chunk_cache.clear_expired();
        
        // Force Rust GC
        std::hint::black_box(vec![0u8; 1]);
        
        Ok(())
    }
}
```

## Concurrency and Parallelism

### 1. Thread Pool Management
```rust
use rayon::ThreadPoolBuilder;

pub struct OptimizedThreading {
    io_pool: rayon::ThreadPool,        // For I/O bound tasks
    cpu_pool: rayon::ThreadPool,       // For CPU bound tasks
    background_pool: rayon::ThreadPool, // For background tasks
}

impl OptimizedThreading {
    pub fn new() -> Result<Self> {
        let cpu_cores = num_cpus::get();
        
        Ok(Self {
            // I/O pool: More threads since they'll be waiting
            io_pool: ThreadPoolBuilder::new()
                .num_threads(cpu_cores * 2)
                .thread_name(|i| format!("digstore-io-{}", i))
                .build()?,
                
            // CPU pool: Match CPU cores
            cpu_pool: ThreadPoolBuilder::new()
                .num_threads(cpu_cores)
                .thread_name(|i| format!("digstore-cpu-{}", i))
                .build()?,
                
            // Background pool: Single thread for housekeeping
            background_pool: ThreadPoolBuilder::new()
                .num_threads(1)
                .thread_name(|_| "digstore-background".to_string())
                .build()?,
        })
    }
    
    pub fn process_files_optimized(&self, files: Vec<PathBuf>) -> Result<Vec<FileEntry>> {
        // Use appropriate thread pool based on operation type
        self.io_pool.install(|| {
            files.par_iter()
                .map(|path| {
                    // Switch to CPU pool for chunking
                    self.cpu_pool.install(|| {
                        self.process_file_with_chunking(path)
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
    }
}
```

### 2. Lock-Free Data Structures
```rust
use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LockFreeProcessing {
    work_queue: SegQueue<PathBuf>,
    result_queue: SegQueue<FileEntry>,
    active_workers: AtomicUsize,
    completed_count: AtomicUsize,
}

impl LockFreeProcessing {
    pub fn process_files_lockfree(&self, files: Vec<PathBuf>) -> Result<Vec<FileEntry>> {
        // Add all files to work queue
        for file in files {
            self.work_queue.push(file);
        }
        
        let total_files = self.work_queue.len();
        
        // Spawn workers
        let workers: Vec<_> = (0..num_cpus::get())
            .map(|_| {
                let work_queue = &self.work_queue;
                let result_queue = &self.result_queue;
                let active_workers = &self.active_workers;
                let completed_count = &self.completed_count;
                
                thread::spawn(move || {
                    active_workers.fetch_add(1, Ordering::SeqCst);
                    
                    while let Some(file_path) = work_queue.pop() {
                        match process_file_fast(&file_path) {
                            Ok(file_entry) => {
                                result_queue.push(file_entry);
                                completed_count.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                eprintln!("Error processing {}: {}", file_path.display(), e);
                            }
                        }
                    }
                    
                    active_workers.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        
        // Wait for completion
        for worker in workers {
            worker.join().unwrap();
        }
        
        // Collect results
        let mut results = Vec::with_capacity(total_files);
        while let Some(entry) = self.result_queue.pop() {
            results.push(entry);
        }
        
        Ok(results)
    }
}
```

## I/O Optimization

### 1. Asynchronous I/O with Batching
```rust
use tokio::fs;
use futures::stream::{FuturesUnordered, StreamExt};

pub struct AsyncBatchIO {
    max_concurrent_reads: usize,
    batch_size: usize,
}

impl AsyncBatchIO {
    pub async fn read_files_async_batched(&self, files: Vec<PathBuf>) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut results = Vec::with_capacity(files.len());
        
        for batch in files.chunks(self.batch_size) {
            let mut futures = FuturesUnordered::new();
            
            // Start concurrent reads for batch
            for path in batch {
                futures.push(self.read_file_with_retry(path.clone()));
            }
            
            // Collect batch results
            while let Some(result) = futures.next().await {
                results.push(result?);
            }
        }
        
        Ok(results)
    }
    
    async fn read_file_with_retry(&self, path: PathBuf) -> Result<(PathBuf, Vec<u8>)> {
        const MAX_RETRIES: usize = 3;
        let mut attempts = 0;
        
        loop {
            match fs::read(&path).await {
                Ok(data) => return Ok((path, data)),
                Err(e) if attempts < MAX_RETRIES => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(10 * attempts as u64)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}
```

### 2. Memory-Mapped File Processing
```rust
use memmap2::{Mmap, MmapOptions};

pub struct MemoryMappedProcessor {
    mmap_threshold: u64,  // Use mmap for files larger than this
    chunk_size: usize,
}

impl MemoryMappedProcessor {
    pub fn process_large_file(&self, path: &Path) -> Result<Vec<Chunk>> {
        let file = File::open(path)?;
        let file_size = file.metadata()?.len();
        
        if file_size > self.mmap_threshold {
            self.process_with_mmap(file, file_size)
        } else {
            self.process_with_streaming(file)
        }
    }
    
    fn process_with_mmap(&self, file: File, file_size: u64) -> Result<Vec<Chunk>> {
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        let mut chunks = Vec::new();
        
        // Process in chunks to avoid large allocations
        for (i, chunk_data) in mmap.chunks(self.chunk_size).enumerate() {
            let offset = i * self.chunk_size;
            let hash = sha256(chunk_data);
            
            chunks.push(Chunk {
                hash,
                offset: offset as u64,
                size: chunk_data.len() as u32,
                data: chunk_data.to_vec(),
            });
        }
        
        Ok(chunks)
    }
    
    async fn process_with_streaming(&self, file: File) -> Result<Vec<Chunk>> {
        let mut reader = BufReader::new(file);
        let mut chunks = Vec::new();
        let mut buffer = vec![0u8; self.chunk_size];
        let mut offset = 0u64;
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 { break; }
            
            let chunk_data = &buffer[..n];
            let hash = sha256(chunk_data);
            
            chunks.push(Chunk {
                hash,
                offset,
                size: n as u32,
                data: chunk_data.to_vec(),
            });
            
            offset += n as u64;
        }
        
        Ok(chunks)
    }
}
```

## Performance Testing Framework

### 1. Comprehensive Benchmarks
```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};

fn bench_small_files_commit(c: &mut Criterion) {
    let mut group = c.benchmark_group("small_files_commit");
    
    // Test different file counts
    for count in [1000, 5000, 10000, 20000] {
        group.throughput(Throughput::Elements(count as u64));
        
        group.bench_with_input(
            BenchmarkId::new("commit", count),
            &count,
            |b, &count| {
                let temp_dir = create_small_test_files(count);
                
                b.iter(|| {
                    let mut store = Store::init(&temp_dir).unwrap();
                    store.add_directory(&temp_dir, true).unwrap();
                    store.commit("Benchmark commit").unwrap()
                });
            },
        );
    }
    
    group.finish();
}

fn bench_large_file_streaming(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_file_streaming");
    
    // Test different large file sizes
    for size_gb in [1, 5, 10] {
        let size_bytes = size_gb * 1024 * 1024 * 1024;
        group.throughput(Throughput::Bytes(size_bytes));
        
        group.bench_with_input(
            BenchmarkId::new("stream_chunk", size_gb),
            &size_bytes,
            |b, &size| {
                let large_file = create_sparse_file(size);
                
                b.iter(|| {
                    chunk_large_file_streaming(&large_file).unwrap()
                });
            },
        );
    }
    
    group.finish();
}

fn bench_mixed_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_workload");
    
    group.bench_function("10k_small_plus_5_large", |b| {
        let temp_dir = TempDir::new().unwrap();
        
        // Create mixed workload
        create_small_files(&temp_dir, 10000, 1024..10240).unwrap();
        create_large_files(&temp_dir, 5, 100 * 1024 * 1024).unwrap();
        
        b.iter(|| {
            let mut store = Store::init(&temp_dir).unwrap();
            store.add_directory(&temp_dir, true).unwrap();
            store.commit("Mixed workload").unwrap()
        });
    });
    
    group.finish();
}

criterion_group!(
    performance_benches,
    bench_small_files_commit,
    bench_large_file_streaming,
    bench_mixed_workload
);
criterion_main!(performance_benches);
```

### 2. Stress Testing
```rust
#[cfg(test)]
mod stress_tests {
    use super::*;
    
    #[test]
    #[ignore] // Run with --ignored for stress testing
    fn stress_test_extreme_file_count() {
        // Test with 100,000 tiny files
        let temp_dir = create_tiny_files(100000, 100..1000);
        let start = Instant::now();
        
        let mut store = Store::init(&temp_dir)?;
        store.add_directory(&temp_dir, true)?;
        let commit_id = store.commit("Extreme file count test")?;
        
        let duration = start.elapsed();
        
        // Should complete in reasonable time
        assert!(duration.as_secs() < 300, "Should complete 100k files in <5 minutes");
        
        // Memory should not explode
        let memory_mb = get_peak_memory_usage() / (1024 * 1024);
        assert!(memory_mb < 1024, "Should use <1GB memory for 100k files");
    }
    
    #[test]
    #[ignore]
    fn stress_test_very_large_file() {
        // Test with 50GB file (sparse)
        let large_file = create_sparse_file(50 * 1024 * 1024 * 1024);
        let start = Instant::now();
        
        let chunks = chunk_large_file_streaming(&large_file)?;
        
        let duration = start.elapsed();
        let throughput_mb_s = (50 * 1024) as f64 / duration.as_secs_f64();
        
        assert!(throughput_mb_s > 400.0, "Should maintain >400 MB/s for very large files");
        
        // Memory should be constant
        let memory_mb = get_peak_memory_usage() / (1024 * 1024);
        assert!(memory_mb < 300, "Should use <300MB memory for 50GB file");
    }
}
```

## Performance Monitoring

### 1. Real-time Metrics Collection
```rust
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct PerformanceMetrics {
    // Counters
    files_processed: AtomicUsize,
    bytes_processed: AtomicU64,
    chunks_created: AtomicUsize,
    chunks_deduplicated: AtomicUsize,
    
    // Timing
    start_time: Instant,
    last_update: Mutex<Instant>,
    
    // Memory tracking
    peak_memory: AtomicU64,
    current_memory: AtomicU64,
}

impl PerformanceMetrics {
    pub fn record_file(&self, size: u64, chunks: usize, dedup_chunks: usize) {
        self.files_processed.fetch_add(1, Ordering::Relaxed);
        self.bytes_processed.fetch_add(size, Ordering::Relaxed);
        self.chunks_created.fetch_add(chunks, Ordering::Relaxed);
        self.chunks_deduplicated.fetch_add(dedup_chunks, Ordering::Relaxed);
        
        // Update memory tracking
        let current = get_current_memory_usage();
        self.current_memory.store(current, Ordering::Relaxed);
        
        let peak = self.peak_memory.load(Ordering::Relaxed);
        if current > peak {
            self.peak_memory.store(current, Ordering::Relaxed);
        }
    }
    
    pub fn get_summary(&self) -> PerformanceSummary {
        let elapsed = self.start_time.elapsed();
        let files = self.files_processed.load(Ordering::Relaxed);
        let bytes = self.bytes_processed.load(Ordering::Relaxed);
        let chunks = self.chunks_created.load(Ordering::Relaxed);
        let dedup = self.chunks_deduplicated.load(Ordering::Relaxed);
        
        PerformanceSummary {
            files_per_second: files as f64 / elapsed.as_secs_f64(),
            mb_per_second: bytes as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0),
            chunks_per_second: chunks as f64 / elapsed.as_secs_f64(),
            deduplication_ratio: dedup as f64 / chunks as f64,
            peak_memory_mb: self.peak_memory.load(Ordering::Relaxed) / (1024 * 1024),
            total_time: elapsed,
        }
    }
}
```

### 2. Performance Alerts
```rust
pub struct PerformanceAlerts {
    thresholds: PerformanceThresholds,
    alert_history: Mutex<VecDeque<Alert>>,
}

pub struct PerformanceThresholds {
    min_files_per_second: f64,
    min_mb_per_second: f64,
    max_memory_mb: u64,
    max_operation_time: Duration,
}

impl PerformanceAlerts {
    pub fn check_performance(&self, metrics: &PerformanceSummary) -> Vec<Alert> {
        let mut alerts = Vec::new();
        
        if metrics.files_per_second < self.thresholds.min_files_per_second {
            alerts.push(Alert::SlowFileProcessing {
                current: metrics.files_per_second,
                expected: self.thresholds.min_files_per_second,
            });
        }
        
        if metrics.mb_per_second < self.thresholds.min_mb_per_second {
            alerts.push(Alert::SlowThroughput {
                current: metrics.mb_per_second,
                expected: self.thresholds.min_mb_per_second,
            });
        }
        
        if metrics.peak_memory_mb > self.thresholds.max_memory_mb {
            alerts.push(Alert::HighMemoryUsage {
                current: metrics.peak_memory_mb,
                limit: self.thresholds.max_memory_mb,
            });
        }
        
        alerts
    }
}

#[derive(Debug)]
pub enum Alert {
    SlowFileProcessing { current: f64, expected: f64 },
    SlowThroughput { current: f64, expected: f64 },
    HighMemoryUsage { current: u64, limit: u64 },
    LongOperationTime { current: Duration, limit: Duration },
}
```

## Configuration for Performance Tuning

### 1. Adaptive Configuration
```toml
[performance]
# Auto-tuning enabled by default
auto_tune = true
performance_monitoring = true

# File size thresholds for different processing strategies
tiny_file_threshold = "4KB"      # Single chunk, no CDC
small_file_threshold = "64KB"    # Fast CDC
medium_file_threshold = "10MB"   # Standard CDC
large_file_threshold = "100MB"   # Streaming CDC
huge_file_threshold = "1GB"      # Memory-mapped processing

# Processing parameters
small_file_batch_size = 200      # Files per batch
large_file_chunk_size = "1MB"    # Chunk size for large files
worker_thread_count = 0          # 0 = auto-detect CPU cores
max_memory_usage = "2GB"         # Maximum memory limit

# I/O parameters
async_io_threshold = "1MB"       # Use async I/O for files larger than this
memory_map_threshold = "10MB"    # Use mmap for files larger than this
read_buffer_size = "64KB"        # Buffer size for streaming reads
write_buffer_size = "64KB"       # Buffer size for streaming writes

# Progress and UI
progress_update_interval = "100ms"  # How often to update progress bars
batch_progress_threshold = 50       # Update progress every N files in batch
quiet_mode_file_threshold = 10000   # Auto-enable quiet mode for large operations
```

### 2. Workload-Specific Profiles
```toml
[profiles.source_code]
# Optimized for source code repositories (many small text files)
description = "Optimized for source code with many small files"
small_file_batch_size = 500
chunking_threshold = "8KB"
deduplication_aggressive = true
compression = "zstd_fast"

[profiles.media_files]
# Optimized for media files (fewer large binary files)
description = "Optimized for media and binary files"
large_file_streaming = true
memory_map_threshold = "5MB"
chunk_size = "2MB"
compression = "zstd_best"

[profiles.mixed_workload]
# Balanced for mixed file sizes
description = "Balanced for mixed small and large files"
adaptive_processing = true
auto_tune_batch_size = true
performance_monitoring = true
memory_limit = "1GB"
```

## Success Criteria

### 1. Performance Benchmarks Must Pass
- ✅ 20,000 small files commit in <60 seconds
- ✅ Large files (>1GB) process at >500 MB/s
- ✅ Memory usage <200MB for large files
- ✅ Memory usage <500MB for 20,000 small files
- ✅ Parallel processing scales with CPU cores
- ✅ Progress feedback doesn't impact performance

### 2. Real-World Validation
- ✅ Linux kernel source tree (70,000+ files) in <3 minutes
- ✅ 10GB video file processes with constant memory
- ✅ Mixed workload (source + media) handles efficiently
- ✅ Repository with 1 million files remains responsive
- ✅ Network file systems work efficiently

### 3. Scalability Requirements
- ✅ Performance scales linearly with CPU cores
- ✅ Memory usage predictable and bounded
- ✅ I/O efficiency maintains high utilization
- ✅ No performance degradation with repository size
- ✅ Graceful handling of resource constraints

This comprehensive performance framework ensures Digstore Min can handle both large files and large numbers of small files with excellent efficiency and user experience.
