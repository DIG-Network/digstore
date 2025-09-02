//! Storage layer for Digstore Min
//!
//! This module handles the storage and retrieval of data in the repository,
//! including layer management, chunking, and file operations.

pub mod adaptive;
pub mod batch;
pub mod binary_staging;
pub mod cache;
pub mod chunk;
pub mod dig_archive;
pub mod layer;
pub mod optimized_staging;
pub mod parallel_processor;
pub mod secure_layer;
pub mod store;
pub mod streaming;

// Re-export commonly used items
pub use adaptive::{AdaptiveProcessor, ProcessingStrategy, WorkloadAnalysis};
pub use batch::{BatchProcessor, BatchResult, OptimizedFileScanner};
pub use binary_staging::{BinaryStagedFile, BinaryStagingArea, StagingStats};
pub use cache::{BufferPool, CacheConfig, ChunkCache};
pub use chunk::{ChunkConfig, ChunkingEngine};
pub use dig_archive::{get_archive_path, ArchiveStats, DigArchive};
pub use layer::Layer;
pub use optimized_staging::OptimizedStagingArea;
pub use parallel_processor::{add_all_parallel, ParallelConfig, ProcessingStats};
pub use secure_layer::SecureLayer;
pub use store::{StagedFile, Store, StoreStatus};
pub use streaming::{FilePointer, StreamingChunkingEngine, StreamingFileEntry};
