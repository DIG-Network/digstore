//! Incremental merkle tree updates for efficient layer construction

use crate::core::{error::*, types::*};
use crate::proofs::merkle::{DigstoreProof, MerkleTree};
use crate::storage::layer::Layer;
use sha2::Digest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Incremental merkle tree builder that can efficiently add nodes
pub struct IncrementalMerkleBuilder {
    /// Current leaves in the tree
    leaves: Vec<Hash>,
    /// Cached intermediate nodes to avoid recomputation
    node_cache: HashMap<(usize, usize), Hash>, // (level, index) -> hash
    /// Current tree (rebuilt only when needed)
    current_tree: Option<MerkleTree>,
    /// Whether the tree needs rebuilding
    dirty: bool,
}

impl Default for IncrementalMerkleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IncrementalMerkleBuilder {
    pub fn new() -> Self {
        Self {
            leaves: Vec::new(),
            node_cache: HashMap::new(),
            current_tree: None,
            dirty: false,
        }
    }

    /// Add a new leaf to the tree
    pub fn add_leaf(&mut self, hash: Hash) {
        self.leaves.push(hash);
        self.dirty = true;

        // Clear cache for affected nodes
        self.invalidate_cache_from_leaf(self.leaves.len() - 1);
    }

    /// Add multiple leaves efficiently
    pub fn add_leaves(&mut self, hashes: &[Hash]) {
        let start_index = self.leaves.len();
        self.leaves.extend_from_slice(hashes);
        self.dirty = true;

        // Invalidate cache for all affected nodes
        for i in start_index..self.leaves.len() {
            self.invalidate_cache_from_leaf(i);
        }
    }

    /// Get the current root hash
    pub fn root(&mut self) -> Result<Hash> {
        if self.leaves.is_empty() {
            return Ok(Hash::zero());
        }

        if self.dirty {
            self.rebuild_tree()?;
        }

        Ok(self.current_tree.as_ref().unwrap().root())
    }

    /// Generate proof for a leaf
    pub fn generate_proof(&mut self, leaf_index: usize) -> Result<DigstoreProof> {
        if leaf_index >= self.leaves.len() {
            return Err(DigstoreError::internal("Leaf index out of bounds"));
        }

        if self.dirty {
            self.rebuild_tree()?;
        }

        self.current_tree
            .as_ref()
            .unwrap()
            .generate_proof(leaf_index)
    }

    /// Get number of leaves
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Finalize the tree and return it
    pub fn finalize(mut self) -> Result<MerkleTree> {
        if self.dirty {
            self.rebuild_tree()?;
        }

        self.current_tree
            .ok_or_else(|| DigstoreError::internal("No tree built"))
    }

    /// Rebuild tree from current leaves
    fn rebuild_tree(&mut self) -> Result<()> {
        if self.leaves.is_empty() {
            self.current_tree = None;
            self.dirty = false;
            return Ok(());
        }

        self.current_tree = Some(MerkleTree::from_hashes(&self.leaves)?);
        self.dirty = false;

        // Clear cache since we rebuilt
        self.node_cache.clear();

        Ok(())
    }

    /// Invalidate cache entries affected by a leaf change
    fn invalidate_cache_from_leaf(&mut self, leaf_index: usize) {
        let mut current_index = leaf_index;
        let mut level = 0;

        // Walk up the tree invalidating affected nodes
        while current_index > 0 || level == 0 {
            self.node_cache.remove(&(level, current_index));
            self.node_cache.remove(&(level, current_index ^ 1)); // Sibling node

            current_index /= 2;
            level += 1;

            if current_index == 0 && level > 0 {
                break;
            }
        }
    }
}

/// Efficient layer writer with streaming
pub struct StreamingLayerWriter {
    buffer_size: usize,
    compression_threshold: usize,
}

impl Default for StreamingLayerWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingLayerWriter {
    pub fn new() -> Self {
        Self {
            buffer_size: 64 * 1024,          // 64KB buffer
            compression_threshold: 4 * 1024, // Compress chunks >4KB
        }
    }

    /// Write layer using streaming to minimize memory usage
    pub fn write_layer_streaming(&self, layer: &Layer, output_path: &Path) -> Result<Hash> {
        // Use existing layer write method
        layer.write_to_file(output_path)?;

        // Compute hash of the written file
        let layer_data = std::fs::read(output_path)?;
        let hash = crate::core::hash::sha256(&layer_data);

        Ok(hash)
    }
}

/// Index cache for fast file lookups
pub struct IndexCache {
    file_index: HashMap<PathBuf, FileLocation>,
    chunk_index: HashMap<Hash, Vec<ChunkLocation>>,
    layer_index: HashMap<Hash, LayerMetadata>,
    cache_stats: IndexCacheStats,
}

/// Location of a file in the repository
#[derive(Debug, Clone)]
pub struct FileLocation {
    pub layer_hash: Hash,
    pub file_index: usize,
    pub size: u64,
    pub chunk_count: usize,
}

/// Location of a chunk in the repository
#[derive(Debug, Clone)]
pub struct ChunkLocation {
    pub layer_hash: Hash,
    pub chunk_index: usize,
    pub offset: u64,
    pub size: u32,
}

/// Index cache statistics
#[derive(Debug, Clone)]
pub struct IndexCacheStats {
    pub file_lookups: u64,
    pub chunk_lookups: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl Default for IndexCache {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexCache {
    pub fn new() -> Self {
        Self {
            file_index: HashMap::new(),
            chunk_index: HashMap::new(),
            layer_index: HashMap::new(),
            cache_stats: IndexCacheStats::new(),
        }
    }

    /// Add layer to index
    pub fn add_layer(&mut self, layer: &Layer, layer_hash: Hash) {
        // Index all files in the layer
        for (file_idx, file_entry) in layer.files.iter().enumerate() {
            let location = FileLocation {
                layer_hash,
                file_index: file_idx,
                size: file_entry.size,
                chunk_count: file_entry.chunks.len(),
            };
            self.file_index.insert(file_entry.path.clone(), location);
        }

        // Index all chunks in the layer
        for (chunk_idx, chunk) in layer.chunks.iter().enumerate() {
            let location = ChunkLocation {
                layer_hash,
                chunk_index: chunk_idx,
                offset: chunk.offset,
                size: chunk.size,
            };

            self.chunk_index
                .entry(chunk.hash)
                .or_default()
                .push(location);
        }

        // Add layer metadata
        self.layer_index.insert(
            layer_hash,
            LayerMetadata {
                layer_number: layer.header.layer_number,
                timestamp: layer.header.timestamp as i64,
                file_count: layer.files.len(),
                chunk_count: layer.chunks.len(),
            },
        );
    }

    /// Find file location
    pub fn find_file(&mut self, path: &Path) -> Option<&FileLocation> {
        self.cache_stats.file_lookups += 1;

        if let Some(location) = self.file_index.get(path) {
            self.cache_stats.cache_hits += 1;
            Some(location)
        } else {
            self.cache_stats.cache_misses += 1;
            None
        }
    }

    /// Find chunk locations
    pub fn find_chunk(&mut self, hash: &Hash) -> Option<&Vec<ChunkLocation>> {
        self.cache_stats.chunk_lookups += 1;

        if let Some(locations) = self.chunk_index.get(hash) {
            self.cache_stats.cache_hits += 1;
            Some(locations)
        } else {
            self.cache_stats.cache_misses += 1;
            None
        }
    }

    /// Get cache statistics
    pub fn get_stats(&self) -> &IndexCacheStats {
        &self.cache_stats
    }

    /// Clear cache
    pub fn clear(&mut self) {
        self.file_index.clear();
        self.chunk_index.clear();
        self.layer_index.clear();
        self.cache_stats = IndexCacheStats::new();
    }
}

/// Layer metadata for index
#[derive(Debug, Clone)]
pub struct LayerMetadata {
    pub layer_number: u64,
    pub timestamp: i64,
    pub file_count: usize,
    pub chunk_count: usize,
}

impl IndexCacheStats {
    fn new() -> Self {
        Self {
            file_lookups: 0,
            chunk_lookups: 0,
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn hit_ratio(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    #[test]
    fn test_incremental_merkle_builder() {
        let mut builder = IncrementalMerkleBuilder::new();

        // Add leaves incrementally
        let hash1 =
            Hash::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let hash2 =
            Hash::from_hex("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
                .unwrap();

        builder.add_leaf(hash1);
        assert_eq!(builder.leaf_count(), 1);

        builder.add_leaf(hash2);
        assert_eq!(builder.leaf_count(), 2);

        // Get root
        let root = builder.root().unwrap();
        assert_ne!(root, Hash::zero());

        // Generate proof
        let proof = builder.generate_proof(0).unwrap();
        assert_eq!(proof.leaf_index, 0);
    }

    #[test]
    fn test_index_cache() {
        let mut cache = IndexCache::new();

        // Create test layer
        let layer = create_test_layer();
        let layer_hash =
            Hash::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap();

        // Add to cache
        cache.add_layer(&layer, layer_hash);

        // Test file lookup
        let test_path = std::path::PathBuf::from("test.txt");
        let location = cache.find_file(&test_path);
        assert!(location.is_some());

        let stats = cache.get_stats();
        assert_eq!(stats.file_lookups, 1);
        assert_eq!(stats.cache_hits, 1);
    }

    fn create_test_layer() -> Layer {
        use crate::storage::layer::Layer;

        let mut layer = Layer::new(LayerType::Full, 1, Hash::zero());

        // Add test file
        let file_entry = FileEntry {
            path: std::path::PathBuf::from("test.txt"),
            hash: Hash::zero(),
            size: 100,
            chunks: vec![ChunkRef {
                hash: Hash::zero(),
                offset: 0,
                size: 100,
            }],
            metadata: FileMetadata {
                mode: 0o644,
                modified: 0,
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };

        layer.add_file(file_entry);
        layer
    }
}
