use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, GenerationState};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Append-only, monotonic root history backed by `roots.log` (§4.3).
/// Line format: `{id}\t{root_hex}\t{timestamp}`.
pub struct RootHistory {
    path: PathBuf,
}

impl RootHistory {
    /// Open (creating if absent) the history file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            std::fs::File::create(&path)?;
        }
        Ok(Self { path })
    }

    /// All generation states, oldest first.
    pub fn entries(&self) -> Result<Vec<GenerationState>> {
        let text = std::fs::read_to_string(&self.path)?;
        let mut out = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split('\t');
            let id = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| StoreError::CorruptStaging("history: bad id".into()))?;
            let root_hex = parts
                .next()
                .ok_or_else(|| StoreError::CorruptStaging("history: missing root".into()))?;
            let root = Bytes32::from_hex(root_hex)
                .map_err(|_| StoreError::CorruptStaging("history: bad root hex".into()))?;
            let timestamp = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| StoreError::CorruptStaging("history: bad timestamp".into()))?;
            out.push(GenerationState {
                id,
                root,
                timestamp,
            });
        }
        Ok(out)
    }

    /// The latest generation, or `None` if the history is empty.
    pub fn head(&self) -> Result<Option<GenerationState>> {
        Ok(self.entries()?.into_iter().last())
    }

    /// The id the next appended generation must use.
    pub fn next_id(&self) -> Result<u64> {
        Ok(match self.head()? {
            Some(h) => h.id + 1,
            None => 0,
        })
    }

    /// Append a generation, enforcing strict monotonic id (`last + 1`, or `0`).
    pub fn append(&mut self, gen: &GenerationState) -> Result<()> {
        let expected = self.next_id()?;
        if gen.id != expected {
            let last = expected.saturating_sub(1);
            return Err(StoreError::NonMonotonicHistory { last, got: gen.id });
        }
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(f, "{}\t{}\t{}", gen.id, gen.root.to_hex(), gen.timestamp)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, GenerationState};
    use tempfile::tempdir;

    fn gs(id: u64, b: u8, ts: u64) -> GenerationState {
        GenerationState {
            id,
            root: Bytes32([b; 32]),
            timestamp: ts,
        }
    }

    #[test]
    fn append_and_read_back_in_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        h.append(&gs(0, 0xa0, 100)).unwrap();
        h.append(&gs(1, 0xa1, 200)).unwrap();

        let all = RootHistory::open(&path).unwrap().entries().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, 0);
        assert_eq!(all[0].root, Bytes32([0xa0; 32]));
        assert_eq!(all[1].id, 1);
        assert_eq!(all[1].timestamp, 200);
    }

    #[test]
    fn first_append_must_be_id_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        let err = h.append(&gs(5, 0xff, 1)).unwrap_err();
        assert!(matches!(
            err,
            crate::StoreError::NonMonotonicHistory { last: _, got: 5 }
        ));
    }

    #[test]
    fn non_consecutive_append_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        h.append(&gs(0, 0x00, 1)).unwrap();
        let err = h.append(&gs(2, 0x02, 2)).unwrap_err();
        assert!(matches!(
            err,
            crate::StoreError::NonMonotonicHistory { last: 0, got: 2 }
        ));
    }

    #[test]
    fn head_returns_latest_generation() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        assert!(h.head().unwrap().is_none());
        h.append(&gs(0, 0x00, 1)).unwrap();
        h.append(&gs(1, 0x11, 2)).unwrap();
        let head = h.head().unwrap().unwrap();
        assert_eq!(head.id, 1);
        assert_eq!(head.root, Bytes32([0x11; 32]));
    }

    #[test]
    fn next_id_is_zero_when_empty_then_increments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        assert_eq!(h.next_id().unwrap(), 0);
        h.append(&gs(0, 0x00, 1)).unwrap();
        assert_eq!(h.next_id().unwrap(), 1);
    }
}
