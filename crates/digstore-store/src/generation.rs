use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, KeyTableEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

fn ser_hash<S: serde::Serializer>(h: &Bytes32, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&h.to_hex())
}

fn de_hash<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Bytes32, D::Error> {
    let s = String::deserialize(d)?;
    Bytes32::from_hex(&s).map_err(|_| serde::de::Error::custom("invalid 32-byte hex"))
}

/// One chunk's placement in the generation: its pool index, content hash, size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub index: u32,
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub hash: Bytes32,
    pub size: u64,
}

/// Manifest key-table record. A deliberate **superset** of the canonical
/// `KeyTableEntry { static_key, generation, chunk_indices, total_size }`:
/// it carries those exact canonical fields plus a human-readable `resource_key`
/// for diff/log. Use `to_key_table_entry` to project to the canonical type the
/// compiler embeds on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTableRecord {
    /// Human-readable resource key (diff/log only; not part of `KeyTableEntry`).
    pub resource_key: String,
    /// Canonical `KeyTableEntry::static_key` — the per-resource lookup key
    /// (= the URN retrieval key for this resource).
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub static_key: Bytes32,
    /// Canonical `KeyTableEntry::generation` — this generation's root.
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub generation: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub total_size: u64,
}

impl KeyTableRecord {
    /// Project to the exact canonical `KeyTableEntry` (drops `resource_key`).
    pub fn to_key_table_entry(&self) -> KeyTableEntry {
        KeyTableEntry {
            static_key: self.static_key,
            generation: self.generation,
            chunk_indices: self.chunk_indices.clone(),
            total_size: self.total_size,
        }
    }
}

/// Generation metadata written to `generations/{root}/manifest.json` (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationManifest {
    pub schema_version: u32,
    pub generation_id: u64,
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub root: Bytes32,
    pub timestamp: u64,
    pub chunks: Vec<ChunkRef>,
    pub key_table: Vec<KeyTableRecord>,
}

impl GenerationManifest {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| StoreError::Manifest(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| StoreError::Manifest(e.to_string()))
    }

    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn read_from(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(&text)
    }

    /// Set of unique chunk hashes in this generation (for diff, §20.4).
    ///
    /// DEVIATION: keyed by the raw `[u8; 32]` (which derives `Ord`) because the
    /// foundation `digstore_core::Bytes32` does NOT derive `Ord`/`PartialOrd`
    /// (the conventions doc requested it but the locked core crate omits it).
    /// Callers recover `Bytes32` with `Bytes32(arr)`.
    pub fn chunk_hashes(&self) -> BTreeSet<[u8; 32]> {
        self.chunks.iter().map(|c| c.hash.0).collect()
    }

    /// Set of resource keys in this generation (for diff, §20.4).
    pub fn resource_keys(&self) -> BTreeSet<String> {
        self.key_table
            .iter()
            .map(|k| k.resource_key.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, KeyTableEntry};
    use tempfile::tempdir;

    fn b(x: u8) -> Bytes32 {
        Bytes32([x; 32])
    }

    fn sample() -> GenerationManifest {
        GenerationManifest {
            schema_version: 1,
            generation_id: 3,
            root: b(0xab),
            timestamp: 1_717_000_000,
            chunks: vec![
                ChunkRef {
                    index: 0,
                    hash: b(0x01),
                    size: 16,
                },
                ChunkRef {
                    index: 1,
                    hash: b(0x02),
                    size: 32,
                },
            ],
            key_table: vec![KeyTableRecord {
                resource_key: "index.html".into(),
                static_key: b(0xff),
                generation: b(0xab),
                chunk_indices: vec![0, 1],
                total_size: 48,
            }],
        }
    }

    #[test]
    fn manifest_roundtrips_through_json() {
        let m = sample();
        let json = m.to_json().unwrap();
        let back = GenerationManifest::from_json(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn manifest_json_uses_hex_for_hashes() {
        let json = sample().to_json().unwrap();
        assert!(json.contains(&"ab".repeat(32))); // root + generation
        assert!(json.contains("\"index.html\""));
        assert!(json.contains("\"generation_id\": 3"));
    }

    #[test]
    fn manifest_writes_and_reads_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let m = sample();
        m.write_to(&path).unwrap();
        let back = GenerationManifest::read_from(&path).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn malformed_json_is_manifest_error() {
        let err = GenerationManifest::from_json("{ not json").unwrap_err();
        assert!(matches!(err, crate::StoreError::Manifest(_)));
    }

    #[test]
    fn invalid_root_hex_is_manifest_error() {
        // Structurally valid JSON, but `root` is not valid 32-byte hex.
        let json = r#"{
            "schema_version": 1,
            "generation_id": 0,
            "root": "zz",
            "timestamp": 1,
            "chunks": [],
            "key_table": []
        }"#;
        let err = GenerationManifest::from_json(json).unwrap_err();
        assert!(matches!(err, crate::StoreError::Manifest(_)));
    }

    #[test]
    fn key_table_record_projects_to_canonical_entry() {
        let rec = KeyTableRecord {
            resource_key: "index.html".into(),
            static_key: b(0xff),
            generation: b(0xab),
            chunk_indices: vec![0, 1],
            total_size: 48,
        };
        let entry: KeyTableEntry = rec.to_key_table_entry();
        assert_eq!(entry.static_key, b(0xff));
        assert_eq!(entry.generation, b(0xab));
        assert_eq!(entry.chunk_indices, vec![0, 1]);
        assert_eq!(entry.total_size, 48);
    }

    #[test]
    fn chunk_and_resource_set_helpers() {
        let m = sample();
        let chunks = m.chunk_hashes();
        assert!(chunks.contains(&b(0x01).0));
        assert!(chunks.contains(&b(0x02).0));
        assert_eq!(chunks.len(), 2);
        let keys = m.resource_keys();
        assert!(keys.contains("index.html"));
        assert_eq!(keys.len(), 1);
    }
}
