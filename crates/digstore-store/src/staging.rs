use crate::error::{Result, StoreError};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One staged resource: its key and the latest bytes staged for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRecord {
    pub resource_key: String,
    pub content: Vec<u8>,
}

/// Append-only binary staging file. Frame (Chia big-endian conventions):
/// `u32 BE key_len | key utf8 | u64 BE content_len | content`.
/// Re-staging a key appends a new frame; read-back is last-write-wins,
/// preserving first-seen order.
pub struct StagingArea {
    path: PathBuf,
}

impl StagingArea {
    /// Open (creating if absent) the staging file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            std::fs::File::create(&path)?;
        }
        Ok(Self { path })
    }

    /// Append a staged resource frame.
    pub fn append(&mut self, resource_key: &str, content: &[u8]) -> Result<()> {
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        let key_bytes = resource_key.as_bytes();
        f.write_all(&(key_bytes.len() as u32).to_be_bytes())?;
        f.write_all(key_bytes)?;
        f.write_all(&(content.len() as u64).to_be_bytes())?;
        f.write_all(content)?;
        Ok(())
    }

    /// Read all frames, collapsing to last-write-wins per key in first-seen order.
    pub fn records(&self) -> Result<Vec<StagedRecord>> {
        let raw = std::fs::read(&self.path)?;
        let mut cursor = 0usize;
        let mut order: Vec<String> = Vec::new();
        let mut latest: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        while cursor < raw.len() {
            let key_len = read_u32(&raw, &mut cursor)? as usize;
            let key = read_bytes(&raw, &mut cursor, key_len)?;
            let key = String::from_utf8(key)
                .map_err(|_| StoreError::CorruptStaging("non-utf8 resource key".into()))?;
            let content_len = read_u64(&raw, &mut cursor)? as usize;
            let content = read_bytes(&raw, &mut cursor, content_len)?;
            if !latest.contains_key(&key) {
                order.push(key.clone());
            }
            latest.insert(key, content);
        }
        Ok(order
            .into_iter()
            .map(|k| StagedRecord {
                content: latest.remove(&k).unwrap(),
                resource_key: k,
            })
            .collect())
    }

    /// True when no records are staged.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.records()?.is_empty())
    }

    /// Truncate the staging file to zero length.
    pub fn clear(&mut self) -> Result<()> {
        std::fs::File::create(&self.path)?;
        Ok(())
    }
}

fn read_u32(buf: &[u8], cursor: &mut usize) -> Result<u32> {
    let end = *cursor + 4;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated u32".into()));
    }
    let v = u32::from_be_bytes(buf[*cursor..end].try_into().unwrap());
    *cursor = end;
    Ok(v)
}

fn read_u64(buf: &[u8], cursor: &mut usize) -> Result<u64> {
    let end = *cursor + 8;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated u64".into()));
    }
    let v = u64::from_be_bytes(buf[*cursor..end].try_into().unwrap());
    *cursor = end;
    Ok(v)
}

fn read_bytes(buf: &[u8], cursor: &mut usize, len: usize) -> Result<Vec<u8>> {
    let end = *cursor + len;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated payload".into()));
    }
    let v = buf[*cursor..end].to_vec();
    *cursor = end;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_one_record_and_read_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("index.html", b"<html>").unwrap();

        let records = StagingArea::open(&path).unwrap().records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_key, "index.html");
        assert_eq!(records[0].content, b"<html>");
    }

    #[test]
    fn last_write_wins_per_key_in_first_seen_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("a.txt", b"old").unwrap();
        area.append("b.txt", b"bee").unwrap();
        area.append("a.txt", b"new").unwrap();

        let records = area.records().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].resource_key, "a.txt");
        assert_eq!(records[0].content, b"new");
        assert_eq!(records[1].resource_key, "b.txt");
        assert_eq!(records[1].content, b"bee");
    }

    #[test]
    fn empty_staging_reads_no_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let area = StagingArea::open(&path).unwrap();
        assert_eq!(area.records().unwrap().len(), 0);
        assert!(area.is_empty().unwrap());
    }

    #[test]
    fn clear_truncates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("a.txt", b"x").unwrap();
        area.clear().unwrap();
        assert!(area.is_empty().unwrap());
    }

    #[test]
    fn truncated_frame_is_reported_corrupt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        // 4-byte length claims 10 bytes of key but none follow.
        std::fs::write(&path, 10u32.to_be_bytes()).unwrap();
        let area = StagingArea::open(&path).unwrap();
        let err = area.records().unwrap_err();
        assert!(matches!(err, StoreError::CorruptStaging(_)));
    }
}
