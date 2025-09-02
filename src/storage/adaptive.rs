//! Adaptive processing system that automatically optimizes for different workloads

use crate::core::{error::*, types::*};
use crate::storage::{
    batch::{BatchProcessor, BatchResult},
    chunk::ChunkingEngine,
    streaming::StreamingChunkingEngine,
};
use indicatif::ProgressBar;
use sha2::Digest;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Adaptive processor that automatically chooses the best strategy
pub struct AdaptiveProcessor {
    streaming_engine: StreamingChunkingEngine,
    batch_processor: BatchProcessor,
    regular_engine: ChunkingEngine,
    workload_analyzer: WorkloadAnalyzer,
    performance_monitor: PerformanceMonitor,
}

/// Workload analysis result
#[derive(Debug, Clone)]
pub struct WorkloadAnalysis {
    pub workload_type: WorkloadType,
    pub small_files: Vec<PathBuf>,
    pub medium_files: Vec<PathBuf>,
    pub large_files: Vec<PathBuf>,
    pub total_size: u64,
    pub estimated_processing_time: Duration,
    pub recommended_strategy: ProcessingStrategy,
}

/// Type of workload detected
#[derive(Debug, Clone, PartialEq)]
pub enum WorkloadType {
    ManySmallFiles,  // >80% files are <64KB
    FewLargeFiles,   // >80% of data in files >10MB
    Mixed,           // Balanced mix of sizes
    SingleLargeFile, // One or few very large files
}

/// Recommended processing strategy
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessingStrategy {
    BatchParallel,  // Batch processing with parallel workers
    StreamingLarge, // Streaming for large files
    Hybrid,         // Mix of strategies
    Individual,     // Process files individually
}

/// Performance monitoring for adaptive tuning
pub struct PerformanceMonitor {
    recent_operations: VecDeque<OperationMetrics>,
    max_history: usize,
    current_config: ProcessingConfig,
}

/// Metrics for a single operation
#[derive(Debug, Clone)]
pub struct OperationMetrics {
    pub strategy: ProcessingStrategy,
    pub file_count: usize,
    pub total_size: u64,
    pub processing_time: Duration,
    pub files_per_second: f64,
    pub mb_per_second: f64,
    pub timestamp: Instant,
}

/// Configuration for processing operations
#[derive(Debug, Clone)]
pub struct ProcessingConfig {
    pub small_file_threshold: u64,
    pub large_file_threshold: u64,
    pub batch_size: usize,
    pub use_parallel: bool,
    pub mmap_threshold: u64,
}

impl AdaptiveProcessor {
    pub fn new() -> Self {
        Self {
            streaming_engine: StreamingChunkingEngine::new(),
            batch_processor: BatchProcessor::new(),
            regular_engine: ChunkingEngine::new(),
            workload_analyzer: WorkloadAnalyzer::new(),
            performance_monitor: PerformanceMonitor::new(),
        }
    }

    /// Process files using adaptive strategy selection
    pub fn process_files_adaptive(
        &mut self,
        files: Vec<PathBuf>,
        progress: Option<&ProgressBar>,
    ) -> Result<AdaptiveProcessingResult> {
        if files.is_empty() {
            return Ok(AdaptiveProcessingResult::empty());
        }

        let start_time = Instant::now();

        // Analyze workload
        let analysis = self.workload_analyzer.analyze_files(&files)?;

        if let Some(pb) = progress {
            pb.set_length(files.len() as u64);
            let message = format!(
                "Processing {} files using {:?} strategy",
                files.len(),
                analysis.recommended_strategy
            );
            pb.set_message(message);
        }

        // Process based on analysis
        let result = match analysis.recommended_strategy {
            ProcessingStrategy::BatchParallel => {
                self.process_batch_parallel(&files, progress, &analysis)
            }
            ProcessingStrategy::StreamingLarge => {
                self.process_streaming_large(&files, progress, &analysis)
            }
            ProcessingStrategy::Hybrid => self.process_hybrid(&files, progress, &analysis),
            ProcessingStrategy::Individual => self.process_individual(&files, progress),
        }?;

        let total_time = start_time.elapsed();

        // Record performance metrics
        let metrics = OperationMetrics {
            strategy: analysis.recommended_strategy,
            file_count: files.len(),
            total_size: analysis.total_size,
            processing_time: total_time,
            files_per_second: files.len() as f64 / total_time.as_secs_f64(),
            mb_per_second: analysis.total_size as f64
                / total_time.as_secs_f64()
                / (1024.0 * 1024.0),
            timestamp: start_time,
        };

        self.performance_monitor.record_operation(metrics);

        // Auto-tune based on performance
        self.performance_monitor.maybe_tune_config();

        Ok(result)
    }

    fn process_batch_parallel(
        &self,
        files: &[PathBuf],
        progress: Option<&ProgressBar>,
        _analysis: &WorkloadAnalysis,
    ) -> Result<AdaptiveProcessingResult> {
        let batch_result = self
            .batch_processor
            .process_files_batch(files.to_vec(), progress)?;

        Ok(AdaptiveProcessingResult {
            file_entries: batch_result.file_entries,
            chunks: batch_result.chunks,
            strategy_used: ProcessingStrategy::BatchParallel,
            performance_metrics: batch_result.performance_metrics,
            deduplication_stats: Some(batch_result.deduplication_stats),
        })
    }

    fn process_streaming_large(
        &self,
        files: &[PathBuf],
        progress: Option<&ProgressBar>,
        _analysis: &WorkloadAnalysis,
    ) -> Result<AdaptiveProcessingResult> {
        let mut file_entries = Vec::new();
        let mut all_chunks = Vec::new();
        let start_time = Instant::now();

        for (i, file_path) in files.iter().enumerate() {
            if let Some(pb) = progress {
                pb.set_position(i as u64);
                let message = format!("Streaming: {}", file_path.display());
                pb.set_message(message);
            }

            let chunks = self.streaming_engine.chunk_file_streaming(file_path)?;
            let file_hash = Self::compute_file_hash_from_chunks(&chunks);
            let file_size = std::fs::metadata(file_path)?.len();

            let file_entry = FileEntry {
                path: file_path.clone(),
                hash: file_hash,
                size: file_size,
                chunks: chunks
                    .iter()
                    .map(|c| ChunkRef {
                        hash: c.hash,
                        offset: c.offset,
                        size: c.size,
                    })
                    .collect(),
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: chrono::Utc::now().timestamp(),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            };

            file_entries.push(file_entry);
            all_chunks.extend(chunks);
        }

        let processing_time = start_time.elapsed();

        Ok(AdaptiveProcessingResult {
            file_entries,
            chunks: all_chunks,
            strategy_used: ProcessingStrategy::StreamingLarge,
            performance_metrics: crate::storage::batch::PerformanceSnapshot {
                files_per_second: files.len() as f64 / processing_time.as_secs_f64(),
                mb_per_second: 0.0, // Would need to calculate
                chunks_per_second: 0.0,
                processing_time,
            },
            deduplication_stats: None,
        })
    }

    fn process_hybrid(
        &self,
        files: &[PathBuf],
        progress: Option<&ProgressBar>,
        analysis: &WorkloadAnalysis,
    ) -> Result<AdaptiveProcessingResult> {
        let mut all_file_entries = Vec::new();
        let mut all_chunks = Vec::new();
        let start_time = Instant::now();

        // Process small files in batch
        if !analysis.small_files.is_empty() {
            if let Some(pb) = progress {
                pb.set_message("Batch processing small files...");
            }

            let batch_result = self
                .batch_processor
                .process_files_batch(analysis.small_files.clone(), progress)?;

            all_file_entries.extend(batch_result.file_entries);
            all_chunks.extend(batch_result.chunks);
        }

        // Process large files with streaming
        if !analysis.large_files.is_empty() {
            if let Some(pb) = progress {
                pb.set_message("Streaming large files...");
            }

            let streaming_result =
                self.process_streaming_large(&analysis.large_files, progress, analysis)?;

            all_file_entries.extend(streaming_result.file_entries);
            all_chunks.extend(streaming_result.chunks);
        }

        // Process medium files individually
        for file_path in &analysis.medium_files {
            let chunks = self.regular_engine.chunk_file_streaming(file_path)?;
            let file_hash = Self::compute_file_hash_from_chunks(&chunks);
            let file_size = std::fs::metadata(file_path)?.len();

            let file_entry = FileEntry {
                path: file_path.clone(),
                hash: file_hash,
                size: file_size,
                chunks: chunks
                    .iter()
                    .map(|c| ChunkRef {
                        hash: c.hash,
                        offset: c.offset,
                        size: c.size,
                    })
                    .collect(),
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: chrono::Utc::now().timestamp(),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            };

            all_file_entries.push(file_entry);
            all_chunks.extend(chunks);
        }

        let processing_time = start_time.elapsed();

        Ok(AdaptiveProcessingResult {
            file_entries: all_file_entries,
            chunks: all_chunks,
            strategy_used: ProcessingStrategy::Hybrid,
            performance_metrics: crate::storage::batch::PerformanceSnapshot {
                files_per_second: files.len() as f64 / processing_time.as_secs_f64(),
                mb_per_second: analysis.total_size as f64
                    / processing_time.as_secs_f64()
                    / (1024.0 * 1024.0),
                chunks_per_second: 0.0,
                processing_time,
            },
            deduplication_stats: None,
        })
    }

    fn process_individual(
        &self,
        files: &[PathBuf],
        progress: Option<&ProgressBar>,
    ) -> Result<AdaptiveProcessingResult> {
        let mut file_entries = Vec::new();
        let mut all_chunks = Vec::new();
        let start_time = Instant::now();

        for (i, file_path) in files.iter().enumerate() {
            if let Some(pb) = progress {
                pb.set_position(i as u64);
                let message = format!("Processing: {}", file_path.display());
                pb.set_message(message);
            }

            let chunks = self.regular_engine.chunk_file_streaming(file_path)?;
            let file_hash = Self::compute_file_hash_from_chunks(&chunks);
            let file_size = std::fs::metadata(file_path)?.len();

            let file_entry = FileEntry {
                path: file_path.clone(),
                hash: file_hash,
                size: file_size,
                chunks: chunks
                    .iter()
                    .map(|c| ChunkRef {
                        hash: c.hash,
                        offset: c.offset,
                        size: c.size,
                    })
                    .collect(),
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: chrono::Utc::now().timestamp(),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            };

            file_entries.push(file_entry);
            all_chunks.extend(chunks);
        }

        let processing_time = start_time.elapsed();

        Ok(AdaptiveProcessingResult {
            file_entries,
            chunks: all_chunks,
            strategy_used: ProcessingStrategy::Individual,
            performance_metrics: crate::storage::batch::PerformanceSnapshot {
                files_per_second: files.len() as f64 / processing_time.as_secs_f64(),
                mb_per_second: 0.0,
                chunks_per_second: 0.0,
                processing_time,
            },
            deduplication_stats: None,
        })
    }

    fn compute_file_hash_from_chunks(chunks: &[Chunk]) -> Hash {
        let mut hasher = sha2::Sha256::new();
        for chunk in chunks {
            hasher.update(&chunk.data);
        }
        Hash::from_bytes(hasher.finalize().into())
    }
}

/// Result of adaptive processing
pub struct AdaptiveProcessingResult {
    pub file_entries: Vec<FileEntry>,
    pub chunks: Vec<Chunk>,
    pub strategy_used: ProcessingStrategy,
    pub performance_metrics: crate::storage::batch::PerformanceSnapshot,
    pub deduplication_stats: Option<crate::storage::batch::DeduplicationStats>,
}

impl AdaptiveProcessingResult {
    fn empty() -> Self {
        Self {
            file_entries: Vec::new(),
            chunks: Vec::new(),
            strategy_used: ProcessingStrategy::Individual,
            performance_metrics: crate::storage::batch::PerformanceSnapshot {
                files_per_second: 0.0,
                mb_per_second: 0.0,
                chunks_per_second: 0.0,
                processing_time: Duration::from_secs(0),
            },
            deduplication_stats: None,
        }
    }
}

/// Workload analyzer
pub struct WorkloadAnalyzer {
    small_file_threshold: u64,
    large_file_threshold: u64,
    batch_threshold: usize,
}

impl WorkloadAnalyzer {
    pub fn new() -> Self {
        Self {
            small_file_threshold: 64 * 1024,        // 64KB
            large_file_threshold: 10 * 1024 * 1024, // 10MB
            batch_threshold: 50,                    // Use batch processing for >50 files
        }
    }

    pub fn analyze_files(&self, files: &[PathBuf]) -> Result<WorkloadAnalysis> {
        let mut small_files = Vec::new();
        let mut medium_files = Vec::new();
        let mut large_files = Vec::new();
        let mut total_size = 0u64;

        // Categorize files by size
        for file_path in files {
            if let Ok(metadata) = std::fs::metadata(file_path) {
                let size = metadata.len();
                total_size += size;

                if size <= self.small_file_threshold {
                    small_files.push(file_path.clone());
                } else if size >= self.large_file_threshold {
                    large_files.push(file_path.clone());
                } else {
                    medium_files.push(file_path.clone());
                }
            }
        }

        // Determine workload type
        let total_files = files.len();
        let small_file_ratio = small_files.len() as f64 / total_files as f64;
        let large_file_size_ratio = if total_size > 0 {
            large_files
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .sum::<u64>() as f64
                / total_size as f64
        } else {
            0.0
        };

        let workload_type = if total_files == 1 && !large_files.is_empty() {
            WorkloadType::SingleLargeFile
        } else if small_file_ratio > 0.8 {
            WorkloadType::ManySmallFiles
        } else if large_file_size_ratio > 0.8 {
            WorkloadType::FewLargeFiles
        } else {
            WorkloadType::Mixed
        };

        // Recommend strategy
        let recommended_strategy = match workload_type {
            WorkloadType::ManySmallFiles if total_files >= self.batch_threshold => {
                ProcessingStrategy::BatchParallel
            }
            WorkloadType::FewLargeFiles | WorkloadType::SingleLargeFile => {
                ProcessingStrategy::StreamingLarge
            }
            WorkloadType::Mixed => ProcessingStrategy::Hybrid,
            _ => ProcessingStrategy::Individual,
        };

        let estimated_processing_time =
            self.estimate_processing_time(&workload_type, total_files, total_size);

        Ok(WorkloadAnalysis {
            workload_type,
            small_files,
            medium_files,
            large_files,
            total_size,
            estimated_processing_time,
            recommended_strategy,
        })
    }

    fn estimate_processing_time(
        &self,
        workload_type: &WorkloadType,
        file_count: usize,
        total_size: u64,
    ) -> Duration {
        // Rough estimates based on workload type
        let estimated_seconds = match workload_type {
            WorkloadType::ManySmallFiles => {
                // Assume 100 files/second for small files
                file_count as f64 / 100.0
            }
            WorkloadType::FewLargeFiles => {
                // Assume 500 MB/s for large files
                total_size as f64 / (500.0 * 1024.0 * 1024.0)
            }
            WorkloadType::Mixed => {
                // Conservative estimate
                file_count as f64 / 50.0
            }
            WorkloadType::SingleLargeFile => {
                // Optimistic for single large file
                total_size as f64 / (800.0 * 1024.0 * 1024.0)
            }
        };

        Duration::from_secs_f64(estimated_seconds.max(1.0))
    }
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        Self {
            recent_operations: VecDeque::new(),
            max_history: 20,
            current_config: ProcessingConfig::default(),
        }
    }

    pub fn record_operation(&mut self, metrics: OperationMetrics) {
        self.recent_operations.push_back(metrics);

        // Keep only recent operations
        while self.recent_operations.len() > self.max_history {
            self.recent_operations.pop_front();
        }
    }

    pub fn maybe_tune_config(&mut self) {
        if self.recent_operations.len() < 5 {
            return; // Need more data
        }

        // Find best performing configuration
        if let Some(best_operation) = self
            .recent_operations
            .iter()
            .max_by(|a, b| a.files_per_second.partial_cmp(&b.files_per_second).unwrap())
        {
            let latest_operation = self.recent_operations.back().unwrap();

            // If current performance is significantly worse, adjust
            if latest_operation.files_per_second < best_operation.files_per_second * 0.8 {
                let best_strategy = best_operation.strategy.clone();
                self.tune_based_on_best_performance(&best_strategy);
            }
        }
    }

    fn tune_based_on_best_performance(&mut self, best_strategy: &ProcessingStrategy) {
        // Adjust configuration based on best performing operation
        match best_strategy {
            ProcessingStrategy::BatchParallel => {
                // Increase batch size if performance was good
                self.current_config.batch_size =
                    (self.current_config.batch_size * 110 / 100).min(1000);
            }
            ProcessingStrategy::StreamingLarge => {
                // Adjust memory mapping threshold
                self.current_config.mmap_threshold =
                    (self.current_config.mmap_threshold * 90 / 100).max(1024 * 1024);
            }
            _ => {}
        }
    }

    pub fn get_current_config(&self) -> &ProcessingConfig {
        &self.current_config
    }

    pub fn get_performance_summary(&self) -> PerformanceSummary {
        if self.recent_operations.is_empty() {
            return PerformanceSummary::empty();
        }

        let total_files: usize = self.recent_operations.iter().map(|op| op.file_count).sum();
        let total_time: Duration = self
            .recent_operations
            .iter()
            .map(|op| op.processing_time)
            .sum();
        let avg_files_per_second = total_files as f64 / total_time.as_secs_f64();

        let best_performance = self
            .recent_operations
            .iter()
            .map(|op| op.files_per_second)
            .fold(0.0, f64::max);

        PerformanceSummary {
            total_operations: self.recent_operations.len(),
            total_files_processed: total_files,
            average_files_per_second: avg_files_per_second,
            best_files_per_second: best_performance,
            total_processing_time: total_time,
        }
    }
}

impl ProcessingConfig {
    fn default() -> Self {
        Self {
            small_file_threshold: 64 * 1024,
            large_file_threshold: 10 * 1024 * 1024,
            batch_size: 200,
            use_parallel: true,
            mmap_threshold: 10 * 1024 * 1024,
        }
    }
}

/// Performance summary
#[derive(Debug, Clone)]
pub struct PerformanceSummary {
    pub total_operations: usize,
    pub total_files_processed: usize,
    pub average_files_per_second: f64,
    pub best_files_per_second: f64,
    pub total_processing_time: Duration,
}

impl PerformanceSummary {
    fn empty() -> Self {
        Self {
            total_operations: 0,
            total_files_processed: 0,
            average_files_per_second: 0.0,
            best_files_per_second: 0.0,
            total_processing_time: Duration::from_secs(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_workload_analyzer() {
        let temp_dir = TempDir::new().unwrap();
        let analyzer = WorkloadAnalyzer::new();

        // Create mixed workload
        let mut files = Vec::new();

        // 10 small files
        for i in 0..10 {
            let path = temp_dir.path().join(format!("small_{}.txt", i));
            fs::write(&path, "small content").unwrap();
            files.push(path);
        }

        // 2 large files (simulated with metadata)
        for i in 0..2 {
            let path = temp_dir.path().join(format!("large_{}.bin", i));
            fs::write(&path, vec![0u8; 100 * 1024]).unwrap(); // 100KB (will be treated as medium)
            files.push(path);
        }

        let analysis = analyzer.analyze_files(&files).unwrap();

        assert_eq!(analysis.workload_type, WorkloadType::ManySmallFiles);
        assert_eq!(analysis.small_files.len(), 10);
        assert!(analysis.total_size > 0);
        assert!(analysis.estimated_processing_time.as_secs() > 0);
    }

    #[test]
    fn test_adaptive_processor() {
        let temp_dir = TempDir::new().unwrap();
        let mut processor = AdaptiveProcessor::new();

        // Create test files
        let mut files = Vec::new();
        for i in 0..5 {
            let path = temp_dir.path().join(format!("test_{}.txt", i));
            fs::write(&path, format!("test content {}", i)).unwrap();
            files.push(path);
        }

        // Process files
        let result = processor.process_files_adaptive(files, None).unwrap();

        assert_eq!(result.file_entries.len(), 5);
        assert!(!result.chunks.is_empty());
        assert!(result.performance_metrics.files_per_second > 0.0);
    }

    #[test]
    fn test_performance_monitor() {
        let mut monitor = PerformanceMonitor::new();

        // Record some operations
        for i in 0..3 {
            let metrics = OperationMetrics {
                strategy: ProcessingStrategy::BatchParallel,
                file_count: 100,
                total_size: 1024 * 1024,
                processing_time: Duration::from_millis(1000 + i * 100),
                files_per_second: 100.0 / (1.0 + i as f64 * 0.1),
                mb_per_second: 1.0,
                timestamp: Instant::now(),
            };
            monitor.record_operation(metrics);
        }

        let summary = monitor.get_performance_summary();
        assert_eq!(summary.total_operations, 3);
        assert_eq!(summary.total_files_processed, 300);
        assert!(summary.average_files_per_second > 0.0);
    }
}
