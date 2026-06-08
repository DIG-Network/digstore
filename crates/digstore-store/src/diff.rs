use crate::generation::GenerationManifest;
use digstore_core::Bytes32;

/// Difference between two generations (§20.4): chunk-set delta + resource-key
/// delta. Results are sorted by hex / lexicographic order for determinism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationDiff {
    pub chunks_added: Vec<Bytes32>,
    pub chunks_removed: Vec<Bytes32>,
    pub keys_added: Vec<String>,
    pub keys_removed: Vec<String>,
}

impl GenerationDiff {
    /// Compute the diff transforming generation `a` into generation `b`.
    ///
    /// DEVIATION: chunk-hash set arithmetic is performed over the raw `[u8; 32]`
    /// (which derives `Ord`) because `digstore_core::Bytes32` does not derive
    /// `Ord`; the deltas are mapped back to `Bytes32` for the public fields.
    pub fn between(a: &GenerationManifest, b: &GenerationManifest) -> Self {
        let a_chunks = a.chunk_hashes();
        let b_chunks = b.chunk_hashes();
        let mut chunks_added: Vec<Bytes32> = b_chunks
            .difference(&a_chunks)
            .map(|h| Bytes32(*h))
            .collect();
        let mut chunks_removed: Vec<Bytes32> = a_chunks
            .difference(&b_chunks)
            .map(|h| Bytes32(*h))
            .collect();
        chunks_added.sort_by_key(|h| h.to_hex());
        chunks_removed.sort_by_key(|h| h.to_hex());

        let a_keys = a.resource_keys();
        let b_keys = b.resource_keys();
        let mut keys_added: Vec<String> = b_keys.difference(&a_keys).cloned().collect();
        let mut keys_removed: Vec<String> = a_keys.difference(&b_keys).cloned().collect();
        keys_added.sort();
        keys_removed.sort();

        Self { chunks_added, chunks_removed, keys_added, keys_removed }
    }

    /// True when the generations have identical chunk sets and resource keys.
    pub fn is_empty(&self) -> bool {
        self.chunks_added.is_empty()
            && self.chunks_removed.is_empty()
            && self.keys_added.is_empty()
            && self.keys_removed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{ChunkRef, GenerationManifest, KeyTableRecord};
    use digstore_core::Bytes32;

    fn b(x: u8) -> Bytes32 {
        Bytes32([x; 32])
    }

    fn gen(id: u64, chunk_bytes: &[u8], keys: &[&str]) -> GenerationManifest {
        GenerationManifest {
            schema_version: 1,
            generation_id: id,
            root: b(id as u8),
            timestamp: id,
            chunks: chunk_bytes
                .iter()
                .enumerate()
                .map(|(i, &c)| ChunkRef { index: i as u32, hash: b(c), size: 1 })
                .collect(),
            key_table: keys
                .iter()
                .map(|k| KeyTableRecord {
                    resource_key: (*k).into(),
                    static_key: b(0xee),
                    generation: b(id as u8),
                    chunk_indices: vec![0],
                    total_size: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn diff_reports_added_and_removed_chunks() {
        let a = gen(0, &[1, 2, 3], &["a.txt"]);
        let b = gen(1, &[2, 3, 4], &["a.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert_eq!(d.chunks_added, vec![Bytes32([4u8; 32])]);
        assert_eq!(d.chunks_removed, vec![Bytes32([1u8; 32])]);
    }

    #[test]
    fn diff_reports_added_and_removed_resource_keys() {
        let a = gen(0, &[1], &["a.txt", "b.txt"]);
        let b = gen(1, &[1], &["b.txt", "c.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert_eq!(d.keys_added, vec!["c.txt".to_string()]);
        assert_eq!(d.keys_removed, vec!["a.txt".to_string()]);
    }

    #[test]
    fn identical_generations_produce_empty_diff() {
        let a = gen(0, &[1, 2], &["a.txt"]);
        let b = gen(1, &[1, 2], &["a.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert!(d.is_empty());
    }
}
