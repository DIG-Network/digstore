//! Storage layer for Digstore Min
//!
//! This module handles the storage and retrieval of data in the repository,
//! including layer management, chunking, and file operations.

pub mod store;
pub mod layer;
pub mod chunk;
pub mod streaming;
pub mod batch;
pub mod optimized_staging;
pub mod adaptive;
pub mod cache;

// Re-export commonly used items
pub use store::{Store, StoreStatus, StagedFile};
pub use layer::Layer;
pub use chunk::{ChunkingEngine, ChunkConfig};
pub use streaming::{StreamingChunkingEngine, StreamingFileEntry, FilePointer};
pub use batch::{BatchProcessor, OptimizedFileScanner, BatchResult};
pub use optimized_staging::{OptimizedStagingArea, StagingStats};
pub use adaptive::{AdaptiveProcessor, WorkloadAnalysis, ProcessingStrategy};
pub use cache::{ChunkCache, BufferPool, CacheConfig};
