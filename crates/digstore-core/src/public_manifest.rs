//! Normalized public manifest — the store's COMPLETE public file surface across
//! ALL roots/generations, one entry per public path holding its LATEST version.
//!
//! Where the per-generation [`crate::datasection`] `KeyTable` lists the resources
//! of a SINGLE generation (and hashes their keys), the public manifest is the
//! flattened, human-path view a consumer reads to see the whole store at a glance:
//! for every public file PATH, which capsule (root) + generation index holds its
//! latest version, that version's content hash, and how many versions of the path
//! exist across the store's history.
//!
//! # Backwards compatibility (HARD RULE, store-format §5.1)
//! This is an ADDITIVE `.dig` section ([`crate::datasection::SectionId::PublicManifest`]
//! = 13). The data-section blob VERSION is unchanged; older readers that do not
//! know section 13 simply ignore it (they get less information, they do not
//! break), and a newer reader treats its ABSENCE as "no public manifest" (an
//! older `.dig`). The body is itself versioned by [`PublicManifest::schema_version`]
//! so future additive fields dispatch on the schema version.
//!
//! ## Field contract (byte-for-byte, cross-repo)
//! Body layout (all integers big-endian; the [`crate::codec`] framing):
//! ```text
//! schema_version : u32
//! entries        : Vec<PublicManifestEntry>  (u32 count, then each entry)
//!   entry:
//!     path             : String   (u32 len + utf8 bytes)
//!     latest_root      : 32 raw bytes
//!     generation_index : u64
//!     sha256_latest    : 32 raw bytes
//!     version_count    : u32
//! ```
//! Entries are ordered ascending by `path` (UTF-8 byte order) so the encoding is
//! deterministic.

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The current [`PublicManifest::schema_version`] new writers emit.
pub const PUBLIC_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// One normalized public path with its latest version + provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PublicManifestEntry {
    /// The public file path (resource key), e.g. `index.html` or `assets/app.js`.
    pub path: String,
    /// The ROOT (capsule) hash of the generation holding this path's latest
    /// version. Serializes as 64-char lowercase hex.
    pub latest_root: Bytes32,
    /// The generation index (id) of that latest version — the ordinal of the
    /// commit that last wrote this path (0-based, matching the store's history).
    pub generation_index: u64,
    /// SHA-256 of the latest version's content: the D5 per-resource leaf,
    /// `SHA-256` over the concatenated ordered chunk ciphertext bodies of the
    /// latest version (the exact per-resource leaf committed in the merkle tree,
    /// which the browser verifier checks). Serializes as 64-char lowercase hex.
    pub sha256_latest: Bytes32,
    /// How many versions of this path exist across the whole store history —
    /// the number of generations (commits) whose file set includes this path.
    pub version_count: u32,
}

impl Encode for PublicManifestEntry {
    fn encode(&self, enc: &mut Encoder) {
        self.path.encode(enc);
        self.latest_root.encode(enc);
        self.generation_index.encode(enc);
        self.sha256_latest.encode(enc);
        self.version_count.encode(enc);
    }
}

impl Decode for PublicManifestEntry {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(PublicManifestEntry {
            path: String::decode(dec)?,
            latest_root: Bytes32::decode(dec)?,
            generation_index: u64::decode(dec)?,
            sha256_latest: Bytes32::decode(dec)?,
            version_count: u32::decode(dec)?,
        })
    }
}

/// The normalized public manifest: every public path's latest version.
///
/// [`entries`](Self::entries) are ordered ascending by `path` (deterministic).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PublicManifest {
    /// Body schema version (currently [`PUBLIC_MANIFEST_SCHEMA_VERSION`]). A
    /// reader dispatches on this; unknown-but-newer bodies remain forward-safe
    /// because fields are only ever appended.
    pub schema_version: u32,
    /// One entry per public path, ascending by `path`.
    pub entries: Vec<PublicManifestEntry>,
}

impl PublicManifest {
    /// Build a manifest from entries, stamping the current schema version and
    /// sorting entries ascending by path (deterministic encoding).
    pub fn new(mut entries: Vec<PublicManifestEntry>) -> Self {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        PublicManifest {
            schema_version: PUBLIC_MANIFEST_SCHEMA_VERSION,
            entries,
        }
    }

    /// Encode the body bytes (the [`crate::datasection::SectionId::PublicManifest`]
    /// section payload).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut enc = Encoder::new();
        self.encode(&mut enc);
        enc.finish()
    }

    /// Decode a body produced by [`to_bytes`](Self::to_bytes).
    pub fn from_bytes(body: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(body);
        Self::decode(&mut dec)
    }

    /// Canonical JSON with hashes as lowercase hex (the machine surface consumers
    /// read). Shape:
    /// `{ "schema_version": u32, "entries": [ { "path", "latest_root",
    /// "generation_index", "sha256_latest", "version_count" } ] }`.
    pub fn to_json(&self) -> String {
        let entries: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path,
                    "latest_root": e.latest_root.to_hex(),
                    "generation_index": e.generation_index,
                    "sha256_latest": e.sha256_latest.to_hex(),
                    "version_count": e.version_count,
                })
            })
            .collect();
        let v = serde_json::json!({
            "schema_version": self.schema_version,
            "entries": entries,
        });
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| String::from("{}"))
    }
}

impl Encode for PublicManifest {
    fn encode(&self, enc: &mut Encoder) {
        self.schema_version.encode(enc);
        self.entries.encode(enc);
    }
}

impl Decode for PublicManifest {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(PublicManifest {
            schema_version: u32::decode(dec)?,
            entries: Vec::<PublicManifestEntry>::decode(dec)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn entry(path: &str, root: u8, gen: u64, sha: u8, vc: u32) -> PublicManifestEntry {
        PublicManifestEntry {
            path: path.to_string(),
            latest_root: Bytes32([root; 32]),
            generation_index: gen,
            sha256_latest: Bytes32([sha; 32]),
            version_count: vc,
        }
    }

    #[test]
    fn new_sorts_entries_by_path() {
        let m = PublicManifest::new(vec![entry("b.txt", 1, 0, 1, 1), entry("a.txt", 2, 1, 2, 1)]);
        assert_eq!(m.schema_version, PUBLIC_MANIFEST_SCHEMA_VERSION);
        assert_eq!(m.entries[0].path, "a.txt");
        assert_eq!(m.entries[1].path, "b.txt");
    }

    #[test]
    fn bytes_round_trip() {
        let m = PublicManifest::new(vec![
            entry("index.html", 0xab, 3, 0xcd, 2),
            entry("assets/app.js", 0x11, 5, 0x22, 4),
        ]);
        let bytes = m.to_bytes();
        let back = PublicManifest::from_bytes(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn empty_round_trips() {
        let m = PublicManifest::new(vec![]);
        let back = PublicManifest::from_bytes(&m.to_bytes()).unwrap();
        assert_eq!(back, m);
        assert!(back.entries.is_empty());
    }

    #[test]
    fn json_uses_hex_and_exact_keys() {
        let m = PublicManifest::new(vec![entry("index.html", 0xab, 3, 0xcd, 2)]);
        let json = m.to_json();
        assert!(json.contains("\"schema_version\""));
        assert!(json.contains("\"path\""));
        assert!(json.contains("\"index.html\""));
        assert!(json.contains("\"latest_root\""));
        assert!(json.contains(&"ab".repeat(32)));
        assert!(json.contains("\"generation_index\""));
        assert!(json.contains("\"sha256_latest\""));
        assert!(json.contains(&"cd".repeat(32)));
        assert!(json.contains("\"version_count\""));
    }

    #[test]
    fn decode_rejects_truncated_body() {
        assert!(PublicManifest::from_bytes(&[0u8, 0, 0]).is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_round_trips_with_hex_hashes() {
        let m = PublicManifest::new(vec![entry("a", 0x01, 7, 0x02, 3)]);
        let s = serde_json::to_string(&m).unwrap();
        // Bytes32 serializes as hex via its serde impl.
        assert!(s.contains(&"01".repeat(32)));
        let back: PublicManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }
}
