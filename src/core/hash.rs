//! Hash utilities for Digstore Min

use crate::core::types::Hash;
use sha2::{Sha256, Digest};
use std::path::Path;
use std::fs::File;
use std::io::{self, Read, BufReader};

/// Compute SHA-256 hash of data
pub fn sha256(data: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(data);
    Hash::from_bytes(hasher.finalize().into())
}

/// Compute SHA-256 hash of bytes (alias for sha256)
pub fn hash_bytes(data: &[u8]) -> Hash {
    sha256(data)
}

/// Compute SHA-256 hash of a string
pub fn hash_string(s: &str) -> Hash {
    sha256(s.as_bytes())
}

/// Compute SHA-256 hash of two hashes (for merkle tree construction)
pub fn hash_pair(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    Hash::from_bytes(hasher.finalize().into())
}

/// Compute SHA-256 hash of a file
pub fn hash_file(path: &Path) -> io::Result<Hash> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(Hash::from_bytes(hasher.finalize().into()))
}

/// Compute SHA-256 hash of multiple chunks of data
pub fn hash_chunks(chunks: &[&[u8]]) -> Hash {
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    Hash::from_bytes(hasher.finalize().into())
}

/// Create a streaming hasher for incremental hashing
pub struct StreamingHasher {
    hasher: Sha256,
}

impl StreamingHasher {
    /// Create a new streaming hasher
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    /// Update the hash with new data
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Finalize the hash and return the result
    pub fn finalize(self) -> Hash {
        Hash::from_bytes(self.hasher.finalize().into())
    }

    /// Reset the hasher to initial state
    pub fn reset(&mut self) {
        self.hasher = Sha256::new();
    }
}

impl Default for StreamingHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_sha256() {
        let data = b"Hello, Digstore!";
        let hash = sha256(data);
        
        // Verify the hash is not zero
        assert_ne!(hash, Hash::zero());
        
        // Verify deterministic
        let hash2 = sha256(data);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hash_string() {
        let s = "Hello, World!";
        let hash = hash_string(s);
        let expected = sha256(s.as_bytes());
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_hash_pair() {
        let hash1 = sha256(b"first");
        let hash2 = sha256(b"second");
        let combined = hash_pair(&hash1, &hash2);
        
        // Should be different from individual hashes
        assert_ne!(combined, hash1);
        assert_ne!(combined, hash2);
        
        // Should be deterministic
        let combined2 = hash_pair(&hash1, &hash2);
        assert_eq!(combined, combined2);
        
        // Order should matter
        let combined_reversed = hash_pair(&hash2, &hash1);
        assert_ne!(combined, combined_reversed);
    }

    #[test]
    fn test_hash_file() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let test_data = b"Test file content for hashing";
        temp_file.write_all(test_data)?;
        temp_file.flush()?;

        let file_hash = hash_file(temp_file.path())?;
        let data_hash = sha256(test_data);
        
        assert_eq!(file_hash, data_hash);
        Ok(())
    }

    #[test]
    fn test_hash_chunks() {
        let chunk1 = b"Hello, ";
        let chunk2 = b"World!";
        let chunks = vec![chunk1.as_slice(), chunk2.as_slice()];
        
        let chunked_hash = hash_chunks(&chunks);
        let combined_data = b"Hello, World!";
        let direct_hash = sha256(combined_data);
        
        assert_eq!(chunked_hash, direct_hash);
    }

    #[test]
    fn test_streaming_hasher() {
        let mut hasher = StreamingHasher::new();
        hasher.update(b"Hello, ");
        hasher.update(b"World!");
        let hash = hasher.finalize();
        
        let direct_hash = sha256(b"Hello, World!");
        assert_eq!(hash, direct_hash);
    }

    #[test]
    fn test_streaming_hasher_reset() {
        let mut hasher = StreamingHasher::new();
        hasher.update(b"Some data");
        hasher.reset();
        hasher.update(b"Different data");
        let hash = hasher.finalize();
        
        let expected = sha256(b"Different data");
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_hash_hex_roundtrip() {
        let original_hash = sha256(b"test data");
        let hex_string = original_hash.to_hex();
        let parsed_hash = Hash::from_hex(&hex_string).unwrap();
        assert_eq!(original_hash, parsed_hash);
    }
}
