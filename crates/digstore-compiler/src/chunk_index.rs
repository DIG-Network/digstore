use std::collections::HashMap;

use digstore_core::Bytes32;

/// Deduplicated global chunk index: maps a chunk's SHA-256 content address to a
/// stable global `u32` index, deduplicating identical chunks across all
/// generations (paper §5.2, §8.3). Insertion order is preserved so the resulting
/// pool layout is deterministic.
#[derive(Debug, Default)]
pub struct ChunkIndex {
    map: HashMap<Bytes32, u32>,
    bodies: Vec<Vec<u8>>,
}

impl ChunkIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a chunk by hash. Returns the existing index if the hash is already
    /// known (dedup), otherwise assigns and returns the next sequential index.
    pub fn insert(&mut self, hash: Bytes32, body: Vec<u8>) -> u32 {
        if let Some(&i) = self.map.get(&hash) {
            return i;
        }
        let i = self.bodies.len() as u32;
        self.map.insert(hash, i);
        self.bodies.push(body);
        i
    }

    /// Look up the global index for a hash, if present.
    pub fn index_of(&self, hash: &Bytes32) -> Option<u32> {
        self.map.get(hash).copied()
    }

    /// Number of unique chunks.
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Chunk bodies in stable insertion (global-index) order.
    pub fn bodies_in_order(&self) -> impl Iterator<Item = &[u8]> {
        self.bodies.iter().map(|b| b.as_slice())
    }
}
