use digstore_core::Bytes32;
use digstore_core::KeyTableEntry;

use crate::chunk_index::ChunkIndex;
use crate::error::{CompilerError, Result};

/// A read-only view of one resource within a generation, so the compiler can
/// consume both `digstore_store` loaded generations and test fixtures.
pub trait ResourceView {
    fn resource_key(&self) -> Bytes32;
    /// (chunk_hash, chunk_body) pairs in resource order.
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)>;
}

/// A read-only view of one loaded generation.
pub trait GenerationView {
    fn root(&self) -> Bytes32;
    fn resources(&self) -> Vec<Box<dyn ResourceView + '_>>;
}

/// Resource-key table: ordered `KeyTableEntry`s. Order is deterministic:
/// generations in load order, resources in their per-generation order.
#[derive(Debug, Default)]
pub struct KeyTable {
    entries: Vec<KeyTableEntry>,
}

impl KeyTable {
    pub fn entries(&self) -> &[KeyTableEntry] {
        &self.entries
    }

    /// First entry whose `static_key` matches `rk`.
    pub fn lookup(&self, rk: &Bytes32) -> Option<&KeyTableEntry> {
        self.entries.iter().find(|e| &e.static_key == rk)
    }

    fn push(&mut self, e: KeyTableEntry) {
        self.entries.push(e);
    }

    /// Integrity check: every chunk index referenced by every entry must be
    /// within `[0, chunk_count)`. Returns `CompilerError::MissingChunk` otherwise.
    pub fn verify_against(&self, chunk_count: u32) -> Result<()> {
        for e in &self.entries {
            for &i in &e.chunk_indices {
                if i >= chunk_count {
                    return Err(CompilerError::MissingChunk(i));
                }
            }
        }
        Ok(())
    }
}

/// Stage 3 + 4 of the pipeline (§5.3): deduplicate chunks across generations into
/// the global `ChunkIndex`, then build the `KeyTable` mapping each resource key to
/// its ordered global chunk indices and reassembled size.
pub fn build_chunk_index_and_key_table<G: GenerationView>(
    generations: &[G],
) -> (ChunkIndex, KeyTable) {
    let mut index = ChunkIndex::new();
    let mut table = KeyTable::default();

    for gen in generations {
        let root = gen.root();
        for resource in gen.resources() {
            let mut chunk_indices = Vec::new();
            let mut total_size: u64 = 0;
            for (hash, body) in resource.chunks() {
                total_size += body.len() as u64;
                let gi = index.insert(hash, body);
                chunk_indices.push(gi);
            }
            table.push(KeyTableEntry {
                static_key: resource.resource_key(),
                generation: root,
                chunk_indices,
                total_size,
            });
        }
    }

    (index, table)
}
