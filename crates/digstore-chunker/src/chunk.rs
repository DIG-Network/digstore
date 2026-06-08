use digstore_core::Bytes32;
use sha2::{Digest, Sha256};

/// A single content-defined chunk: its raw bytes, the byte offset of its first
/// byte within the original input, and its SHA-256 content address (paper §8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// SHA-256 of `data` — the chunk's content address.
    pub hash: Bytes32,
    /// The raw chunk bytes.
    pub data: Vec<u8>,
    /// Byte offset of this chunk's first byte within the original input.
    pub offset: usize,
}

impl Chunk {
    /// Build a chunk from its offset and raw bytes, computing the SHA-256 address.
    pub fn new(offset: usize, data: Vec<u8>) -> Self {
        let hash = hash_data(&data);
        Chunk { hash, data, offset }
    }

    /// Length of the chunk in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the chunk is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// SHA-256 content address of a byte slice.
pub fn hash_data(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Bytes32(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_hashes_empty_data() {
        let c = Chunk::new(0, Vec::new());
        assert_eq!(
            c.hash.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(c.offset, 0);
        assert!(c.data.is_empty());
    }

    #[test]
    fn chunk_hashes_abc() {
        let c = Chunk::new(7, b"abc".to_vec());
        assert_eq!(
            c.hash.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(c.offset, 7);
        assert_eq!(c.data, b"abc");
    }

    #[test]
    fn chunk_len_reports_data_length() {
        let c = Chunk::new(0, vec![1, 2, 3, 4]);
        assert_eq!(c.len(), 4);
        assert!(!c.is_empty());
    }

    #[test]
    fn hash_data_matches_chunk_hash() {
        let data = vec![9u8, 8, 7, 6, 5];
        let c = Chunk::new(0, data.clone());
        assert_eq!(c.hash, hash_data(&data));
    }
}
