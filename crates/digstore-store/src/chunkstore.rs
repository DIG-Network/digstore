use crate::error::{Result, StoreError};
use digstore_core::Bytes32;
use std::path::{Path, PathBuf};

/// Per-directory content-addressed, write-once chunk store. One file per unique
/// chunk, named by lower-case hex of its SHA-256 hash. A repeated `put` is a
/// no-op (deduplication within this directory, §8.2).
pub struct ChunkStore {
    chunks_dir: PathBuf,
}

impl ChunkStore {
    /// Create over an existing or to-be-created chunks directory.
    pub fn new(chunks_dir: impl AsRef<Path>) -> Self {
        Self {
            chunks_dir: chunks_dir.as_ref().to_path_buf(),
        }
    }

    fn chunk_path(&self, hash: Bytes32) -> PathBuf {
        self.chunks_dir.join(hash.to_hex())
    }

    /// Store `data` under `hash`. Returns `true` if newly written, `false` if it
    /// already existed (deduplicated).
    pub fn put(&self, hash: Bytes32, data: &[u8]) -> Result<bool> {
        std::fs::create_dir_all(&self.chunks_dir)?;
        let path = self.chunk_path(hash);
        if path.exists() {
            return Ok(false);
        }
        // Atomic-ish: write to temp then rename within the same dir.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)?;
        Ok(true)
    }

    /// True if a chunk with this hash is present in this directory.
    pub fn contains(&self, hash: Bytes32) -> Result<bool> {
        Ok(self.chunk_path(hash).exists())
    }

    /// Read a chunk's bytes.
    pub fn get(&self, hash: Bytes32) -> Result<Vec<u8>> {
        let path = self.chunk_path(hash);
        if !path.exists() {
            return Err(StoreError::ChunkNotFound(hash.to_hex()));
        }
        Ok(std::fs::read(&path)?)
    }

    /// Number of unique chunk files present.
    pub fn count(&self) -> Result<usize> {
        if !self.chunks_dir.exists() {
            return Ok(0);
        }
        let mut n = 0;
        for entry in std::fs::read_dir(&self.chunks_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            // Ignore stray .tmp files left by an interrupted write.
            if name.to_string_lossy().ends_with(".tmp") {
                continue;
            }
            if entry.file_type()?.is_file() {
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;
    use tempfile::tempdir;

    fn h(b: u8) -> Bytes32 {
        Bytes32([b; 32])
    }

    #[test]
    fn put_writes_chunk_file_named_by_hash() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        let wrote = cs.put(h(0xaa), b"chunk-bytes").unwrap();
        assert!(wrote, "first put writes");
        let file = dir.path().join("aa".repeat(32));
        assert!(file.exists());
        assert_eq!(std::fs::read(&file).unwrap(), b"chunk-bytes");
    }

    #[test]
    fn duplicate_put_is_noop_and_returns_false() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        assert!(cs.put(h(0xbb), b"data").unwrap());
        let wrote_again = cs.put(h(0xbb), b"data").unwrap();
        assert!(!wrote_again, "second put deduplicates");
        assert_eq!(cs.count().unwrap(), 1);
    }

    #[test]
    fn count_reflects_unique_chunks() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        cs.put(h(1), b"a").unwrap();
        cs.put(h(2), b"b").unwrap();
        cs.put(h(1), b"a").unwrap(); // dup
        cs.put(h(3), b"c").unwrap();
        assert_eq!(cs.count().unwrap(), 3);
    }

    #[test]
    fn contains_and_get_roundtrip() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        assert!(!cs.contains(h(9)).unwrap());
        cs.put(h(9), b"nine").unwrap();
        assert!(cs.contains(h(9)).unwrap());
        assert_eq!(cs.get(h(9)).unwrap(), b"nine");
    }
}
