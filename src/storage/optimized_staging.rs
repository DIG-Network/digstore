//! Optimized staging area for efficient bulk operations

use crate::core::{error::*, types::*};
use crate::storage::StagedFile;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Optimized staging area using IndexMap for ordered iteration with O(1) lookups
pub struct OptimizedStagingArea {
    /// Main staging storage with insertion order preservation
    staged_files: IndexMap<PathBuf, StagedFile>,
    /// Batch operations for disk persistence
    pending_writes: VecDeque<StagingBatch>,
    /// Configuration
    batch_size: usize,
    write_threshold: Duration,
    last_write: Instant,
    /// Dirty tracking for incremental persistence
    dirty_files: indexmap::IndexSet<PathBuf>,
}

/// A batch of staging operations for efficient persistence
#[derive(Serialize, Deserialize)]
pub struct StagingBatch {
    batch_id: uuid::Uuid,
    files: Vec<(PathBuf, StagedFile)>,
    timestamp: i64,
    checksum: Hash,
}

/// Bulk staging operation result
pub struct BulkStagingResult {
    pub files_added: usize,
    pub total_size: u64,
    pub processing_time: Duration,
    pub persistence_time: Duration,
}

impl OptimizedStagingArea {
    pub fn new() -> Self {
        Self {
            staged_files: IndexMap::new(),
            pending_writes: VecDeque::new(),
            batch_size: 1000,                        // Persist every 1000 files
            write_threshold: Duration::from_secs(5), // Or every 5 seconds
            last_write: Instant::now(),
            dirty_files: indexmap::IndexSet::new(),
        }
    }

    pub fn with_config(batch_size: usize, write_threshold: Duration) -> Self {
        Self {
            staged_files: IndexMap::new(),
            pending_writes: VecDeque::new(),
            batch_size,
            write_threshold,
            last_write: Instant::now(),
            dirty_files: indexmap::IndexSet::new(),
        }
    }

    /// Add multiple files in a single bulk operation
    pub fn add_files_bulk(
        &mut self,
        files: Vec<(PathBuf, StagedFile)>,
    ) -> Result<BulkStagingResult> {
        let start_time = Instant::now();

        // Reserve capacity to avoid reallocations
        self.staged_files.reserve(files.len());
        self.dirty_files.reserve(files.len());

        let mut total_size = 0u64;

        // Add all files to staging area
        for (path, staged_file) in files {
            total_size += staged_file.file_entry.size;
            self.staged_files.insert(path.clone(), staged_file);
            self.dirty_files.insert(path);
        }

        let processing_time = start_time.elapsed();

        // Check if we should persist
        let persistence_start = Instant::now();
        if self.should_persist() {
            self.create_persistence_batch()?;
        }
        let persistence_time = persistence_start.elapsed();

        Ok(BulkStagingResult {
            files_added: self.dirty_files.len(),
            total_size,
            processing_time,
            persistence_time,
        })
    }

    /// Add a single file (optimized for bulk additions)
    pub fn add_file(&mut self, path: PathBuf, staged_file: StagedFile) -> Result<()> {
        self.staged_files.insert(path.clone(), staged_file);
        self.dirty_files.insert(path);

        // Check if we should persist
        if self.should_persist() {
            self.create_persistence_batch()?;
        }

        Ok(())
    }

    /// Get a staged file
    pub fn get_file(&self, path: &Path) -> Option<&StagedFile> {
        self.staged_files.get(path)
    }

    /// Check if a file is staged
    pub fn contains_file(&self, path: &Path) -> bool {
        self.staged_files.contains_key(path)
    }

    /// Remove a file from staging
    pub fn remove_file(&mut self, path: &Path) -> Option<StagedFile> {
        self.dirty_files.insert(path.to_path_buf());
        self.staged_files.remove(path)
    }

    /// Get all staged files (preserves insertion order)
    pub fn get_all_files(&self) -> impl Iterator<Item = (&PathBuf, &StagedFile)> {
        self.staged_files.iter()
    }

    /// Get staging statistics
    pub fn get_stats(&self) -> StagingStats {
        let total_size = self.staged_files.values().map(|f| f.file_entry.size).sum();

        let total_chunks = self.staged_files.values().map(|f| f.chunks.len()).sum();

        StagingStats {
            file_count: self.staged_files.len(),
            total_size,
            total_chunks,
            dirty_files: self.dirty_files.len(),
            pending_batches: self.pending_writes.len(),
        }
    }

    /// Commit all staged files and clear staging
    pub fn commit_all(&mut self) -> Result<Vec<(PathBuf, StagedFile)>> {
        // Ensure all pending writes are completed
        self.flush_pending_writes()?;

        // Return all staged files in insertion order
        let files: Vec<_> = self.staged_files.drain(..).collect();
        self.dirty_files.clear();

        // Clear staging on disk
        self.clear_staging_files()?;

        Ok(files)
    }

    /// Clear all staged files
    pub fn clear(&mut self) -> Result<()> {
        self.staged_files.clear();
        self.dirty_files.clear();
        self.pending_writes.clear();
        self.clear_staging_files()?;
        Ok(())
    }

    fn should_persist(&self) -> bool {
        self.dirty_files.len() >= self.batch_size
            || self.last_write.elapsed() >= self.write_threshold
    }

    fn create_persistence_batch(&mut self) -> Result<()> {
        if self.dirty_files.is_empty() {
            return Ok(());
        }

        // Create batch for background persistence
        let batch_files: Vec<_> = self
            .dirty_files
            .iter()
            .filter_map(|path| {
                self.staged_files
                    .get(path)
                    .map(|f| (path.clone(), f.clone()))
            })
            .collect();

        if batch_files.is_empty() {
            return Ok(());
        }

        let batch = StagingBatch {
            batch_id: uuid::Uuid::new_v4(),
            files: batch_files,
            timestamp: chrono::Utc::now().timestamp(),
            checksum: self.compute_batch_checksum(),
        };

        // Add to pending writes
        self.pending_writes.push_back(batch);
        self.dirty_files.clear();
        self.last_write = Instant::now();

        // Limit pending batches to avoid memory growth
        while self.pending_writes.len() > 10 {
            self.pending_writes.pop_front();
        }

        Ok(())
    }

    fn flush_pending_writes(&mut self) -> Result<()> {
        // In a real implementation, this would write batches to disk
        // For now, just clear the pending writes
        self.pending_writes.clear();
        Ok(())
    }

    fn clear_staging_files(&self) -> Result<()> {
        // In a real implementation, this would remove staging files from disk
        Ok(())
    }

    fn compute_batch_checksum(&self) -> Hash {
        // Compute checksum of all staged files for integrity
        let mut hasher = sha2::Sha256::new();
        for (path, staged_file) in &self.staged_files {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(staged_file.file_entry.hash.as_bytes());
        }
        Hash::from_bytes(hasher.finalize().into())
    }
}

/// Statistics about the staging area
#[derive(Debug, Clone)]
pub struct StagingStats {
    pub file_count: usize,
    pub total_size: u64,
    pub total_chunks: usize,
    pub dirty_files: usize,
    pub pending_batches: usize,
}

/// Efficient file path interning to reduce memory usage
pub struct PathInterner {
    paths: IndexMap<String, u32>,
    reverse_map: Vec<String>,
    next_id: u32,
}

impl PathInterner {
    pub fn new() -> Self {
        Self {
            paths: IndexMap::new(),
            reverse_map: Vec::new(),
            next_id: 0,
        }
    }

    /// Intern a path string and return its ID
    pub fn intern(&mut self, path: &str) -> u32 {
        if let Some(&id) = self.paths.get(path) {
            id
        } else {
            let id = self.next_id;
            self.paths.insert(path.to_string(), id);
            self.reverse_map.push(path.to_string());
            self.next_id += 1;
            id
        }
    }

    /// Get path string by ID
    pub fn get_path(&self, id: u32) -> Option<&str> {
        self.reverse_map.get(id as usize).map(|s| s.as_str())
    }

    /// Get statistics
    pub fn stats(&self) -> InternerStats {
        let total_chars: usize = self.reverse_map.iter().map(|s| s.len()).sum();
        let average_length = if self.reverse_map.is_empty() {
            0.0
        } else {
            total_chars as f64 / self.reverse_map.len() as f64
        };

        InternerStats {
            unique_paths: self.paths.len(),
            total_characters: total_chars,
            average_path_length: average_length,
            memory_saved_estimate: self.estimate_memory_saved(),
        }
    }

    fn estimate_memory_saved(&self) -> usize {
        // Estimate memory saved by interning vs storing full paths
        let full_path_memory: usize = self.reverse_map.iter().map(|s| s.len() + 24).sum(); // String overhead
        let interned_memory = self.reverse_map.len() * 4 + // IDs
                             self.reverse_map.iter().map(|s| s.len()).sum::<usize>(); // Unique strings

        full_path_memory.saturating_sub(interned_memory)
    }
}

/// Statistics about path interning
#[derive(Debug, Clone)]
pub struct InternerStats {
    pub unique_paths: usize,
    pub total_characters: usize,
    pub average_path_length: f64,
    pub memory_saved_estimate: usize,
}

/// Compact staging entry for memory efficiency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactStagedFile {
    pub path_id: u32, // Reference to interned path
    pub file_hash: Hash,
    pub file_size: u64,
    pub chunk_count: u16,
    pub first_chunk_hash: Hash,
    pub is_staged: bool,
    pub timestamp: i64,
}

impl CompactStagedFile {
    pub fn from_staged_file(staged_file: &StagedFile, path_id: u32) -> Self {
        let first_chunk_hash = staged_file
            .chunks
            .first()
            .map(|c| c.hash)
            .unwrap_or_else(Hash::zero);

        Self {
            path_id,
            file_hash: staged_file.file_entry.hash,
            file_size: staged_file.file_entry.size,
            chunk_count: staged_file.chunks.len() as u16,
            first_chunk_hash,
            is_staged: staged_file.is_staged,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_optimized_staging_bulk_operations() {
        let mut staging = OptimizedStagingArea::new();

        // Create test staged files
        let mut files = Vec::new();
        for i in 0..100 {
            let path = PathBuf::from(format!("test_{:03}.txt", i));
            let staged_file = create_test_staged_file(&path, i as u64);
            files.push((path, staged_file));
        }

        // Test bulk addition
        let result = staging.add_files_bulk(files).unwrap();

        assert_eq!(result.files_added, 100);
        assert!(result.total_size > 0);
        assert!(result.processing_time.as_millis() < 100); // Should be fast

        // Test retrieval
        let test_path = PathBuf::from("test_050.txt");
        assert!(staging.contains_file(&test_path));

        let staged_file = staging.get_file(&test_path).unwrap();
        assert_eq!(staged_file.file_entry.size, 50);

        // Test stats
        let stats = staging.get_stats();
        assert_eq!(stats.file_count, 100);
        assert_eq!(stats.total_size, (0..100).sum::<u64>());
    }

    #[test]
    fn test_path_interner() {
        let mut interner = PathInterner::new();

        // Test interning
        let id1 = interner.intern("src/main.rs");
        let id2 = interner.intern("src/lib.rs");
        let id3 = interner.intern("src/main.rs"); // Duplicate

        assert_eq!(id1, id3); // Same path should get same ID
        assert_ne!(id1, id2); // Different paths get different IDs

        // Test retrieval
        assert_eq!(interner.get_path(id1), Some("src/main.rs"));
        assert_eq!(interner.get_path(id2), Some("src/lib.rs"));

        // Test stats
        let stats = interner.stats();
        assert_eq!(stats.unique_paths, 2);
        assert!(stats.memory_saved_estimate > 0);
    }

    #[test]
    fn test_compact_staged_file() {
        let path = PathBuf::from("test.txt");
        let staged_file = create_test_staged_file(&path, 1024);

        let compact = CompactStagedFile::from_staged_file(&staged_file, 42);

        assert_eq!(compact.path_id, 42);
        assert_eq!(compact.file_size, 1024);
        assert_eq!(compact.chunk_count, 1);
        assert!(compact.is_staged);
    }

    #[test]
    fn test_staging_persistence_batching() {
        let mut staging = OptimizedStagingArea::with_config(10, Duration::from_millis(100));

        // Add files one by one
        for i in 0..15 {
            let path = PathBuf::from(format!("file_{}.txt", i));
            let staged_file = create_test_staged_file(&path, i);
            staging.add_file(path, staged_file).unwrap();
        }

        // Should have created at least one batch (>10 files)
        assert!(!staging.pending_writes.is_empty());

        // Test commit
        let committed_files = staging.commit_all().unwrap();
        assert_eq!(committed_files.len(), 15);
        assert!(staging.staged_files.is_empty());
    }

    fn create_test_staged_file(path: &Path, size: u64) -> StagedFile {
        let chunk = Chunk {
            hash: crate::core::hash::sha256(&size.to_le_bytes()),
            offset: 0,
            size: size as u32,
            data: vec![0u8; size as usize],
        };

        StagedFile {
            file_entry: FileEntry {
                path: path.to_path_buf(),
                hash: chunk.hash,
                size,
                chunks: vec![ChunkRef {
                    hash: chunk.hash,
                    offset: 0,
                    size: size as u32,
                }],
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: chrono::Utc::now().timestamp(),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            },
            chunks: vec![chunk],
            is_staged: true,
        }
    }
}
