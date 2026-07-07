//! CHIP-0007 NFT metadata builder + validator (roadmap #36).
//!
//! CHIP-0007 is the off-chain NFT metadata JSON standard — the document an NFT's `metadata_uris`
//! point at. The on-chain NFT coin pins three SHA-256 hashes (`data_hash`, `metadata_hash`,
//! `license_hash`) that MUST equal the hash of the bytes the corresponding URIs actually serve.
//!
//! This module is the single, shared, tested implementation of: generating canonical CHIP-0007
//! JSON, computing those hashes FROM bytes, and validating URI↔hash agreement + schema. It exists
//! to kill the footgun of every consumer (CLI `nft mint`, hub badge minting, the SDK) hand-rolling
//! SHA-256 and trusting raw input. Pure data + hashing — no chain, no keys.
//!
//! ## Byte-for-byte contract with `chip35_dl_coin`
//! This is a deliberate **mirror** of `chip35_dl_coin`'s `core/src/metadata.rs`: the struct field
//! order, the `serde(skip_serializing_if)` rules, the compact `serde_json::to_string` rendering, and
//! the `sha256` primitive (`chia_sha2::Sha256`) are all identical, so the canonical JSON — and
//! therefore the `metadata_hash` pinned on-chain — is byte-identical whether it is computed by the
//! Rust CLI here, the chip35 wasm, or the hub. A drift in EITHER repo breaks NFT verification across
//! the ecosystem; change them together (SYSTEM.md → CHIP-0007 metadata contract).

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

/// The canonical CHIP-0007 `format` discriminator value.
pub const CHIP0007_FORMAT: &str = "CHIP-0007";

/// Errors from CHIP-0007 metadata building/validation. Distinct from [`ChainError`] so the CLI can
/// surface actionable, metadata-specific messages (and so this module stays chain-agnostic).
///
/// [`ChainError`]: crate::ChainError
#[derive(Debug, ThisError, PartialEq, Eq)]
pub enum MetadataError {
    /// `format` is not `"CHIP-0007"`.
    #[error("invalid format: expected \"{CHIP0007_FORMAT}\", got {0:?}")]
    BadFormat(String),

    /// A required field (e.g. `name`) is missing or empty.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// A computed hash does not equal the on-chain hash for the same asset (URI↔hash disagreement).
    /// `which` is "data" | "metadata" | "license".
    #[error("{which} hash mismatch: bytes hash to {computed} but on-chain hash is {expected}")]
    HashMismatch {
        which: &'static str,
        computed: String,
        expected: String,
    },

    /// `series_number > series_total`.
    #[error("series_number ({number}) exceeds series_total ({total})")]
    BadSeries { number: u64, total: u64 },

    /// JSON (de)serialization failed.
    #[error("json error: {0}")]
    Json(String),
}

/// A single CHIP-0007 attribute (trait) on an NFT **item**.
///
/// Distinct from [`CollectionAttribute`] (issue #187): per CHIP-0007, an NFT item's `attributes`
/// use the field name `trait_type`; a *collection's* `attributes` use `type`. Reusing one struct
/// for both was the #187 bug (a conformant collection.json's `type` field was rejected because
/// this struct demands `trait_type`). Keep item attributes on `trait_type` — do not merge the two
/// shapes back together.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attribute {
    /// The trait category (e.g. `"Background"`).
    pub trait_type: String,
    /// The trait value (e.g. `"Blue"`). Stored as a string for byte-stable hashing; numeric
    /// values are the caller's responsibility to stringify consistently.
    pub value: String,
}

/// A single CHIP-0007 **collection-level** attribute (icon/banner/website/twitter/etc).
///
/// Per CHIP-0007, collection-level attributes use `type` as the field name — DISTINCT from an NFT
/// item's `attributes`, which use `trait_type` ([`Attribute`]). Issue #187: a prior version of
/// this codebase wrongly reused [`Attribute`] (with `trait_type`) for collection attributes too,
/// rejecting every conformant collection.json. This type serializes with `type` going forward;
/// on READ it also accepts the legacy `trait_type` spelling (`#[serde(alias)]`, §5.1 back-compat)
/// so already-emitted DIG collection.json documents keep parsing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectionAttribute {
    /// The attribute category (e.g. `"icon"`, `"banner"`, `"website"`). Serializes as CHIP-0007's
    /// `type`; also accepts the older, non-conformant `trait_type` spelling on read.
    #[serde(rename = "type", alias = "trait_type")]
    pub kind: String,
    /// The attribute value (e.g. a URI).
    pub value: String,
}

/// The collection block embedded in a CHIP-0007 item, linking the item to its [`Collection`].
///
/// [`Collection`]: crate::collection::Collection
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectionRef {
    /// The collection id (stable across all items in the collection).
    pub id: String,
    /// The human-readable collection name.
    pub name: String,
    /// Collection-level attributes (icon/banner/website/etc), as CHIP-0007 `type`/`value` pairs
    /// ([`CollectionAttribute`] — NOT [`Attribute`]; see #187).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<CollectionAttribute>,
}

/// A CHIP-0007 metadata document (the off-chain JSON an NFT's `metadata_uris` point at).
///
/// Serializes to deterministic JSON (fixed field order, empty optionals omitted) so
/// [`compute_metadata_hash`](Self::compute_metadata_hash) is reproducible byte-for-byte across
/// callers — a requirement, because that hash is pinned on-chain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chip0007Metadata {
    /// MUST be `"CHIP-0007"`.
    pub format: String,
    /// The NFT name. Required.
    pub name: String,
    /// Free-text description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the content is sensitive/NSFW.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive_content: bool,
    /// The collection this item belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionRef>,
    /// Per-item traits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<Attribute>,
    /// 1-based position of this item within its series/collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_number: Option<u64>,
    /// Total items in the series/collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_total: Option<u64>,
    /// The tool that minted this (e.g. `"DIG"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minting_tool: Option<String>,
}

impl Chip0007Metadata {
    /// Build a minimal valid CHIP-0007 document with the canonical `format` set.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: CHIP0007_FORMAT.to_string(),
            name: name.into(),
            description: None,
            sensitive_content: false,
            collection: None,
            attributes: Vec::new(),
            series_number: None,
            series_total: None,
            minting_tool: None,
        }
    }

    /// Serialize to canonical (deterministic) JSON bytes. Field order is fixed by the struct
    /// definition and empty optionals are omitted, so two callers building the same logical
    /// document produce byte-identical JSON — and therefore the same [`compute_metadata_hash`].
    ///
    /// [`compute_metadata_hash`]: Self::compute_metadata_hash
    pub fn to_canonical_json(&self) -> Result<String, MetadataError> {
        serde_json::to_string(self).map_err(|e| MetadataError::Json(e.to_string()))
    }

    /// Compute `metadata_hash = sha256(canonical_json_bytes)` — the value pinned on-chain in the
    /// NFT's `metadata_hash`. Never hand-roll this; mismatches silently break verification.
    pub fn compute_metadata_hash(&self) -> Result<Bytes32, MetadataError> {
        Ok(sha256(self.to_canonical_json()?.as_bytes()))
    }

    /// Validate the document's schema (roadmap #36's "validates schema"):
    /// `format == "CHIP-0007"`, `name` non-empty, and `series_number <= series_total`.
    ///
    /// This is the cheap structural check. Use [`validate_uri_hash`] to additionally confirm a
    /// hash matches real bytes.
    pub fn validate_schema(&self) -> Result<(), MetadataError> {
        if self.format != CHIP0007_FORMAT {
            return Err(MetadataError::BadFormat(self.format.clone()));
        }
        if self.name.trim().is_empty() {
            return Err(MetadataError::MissingField("name"));
        }
        if let (Some(number), Some(total)) = (self.series_number, self.series_total) {
            if number > total {
                return Err(MetadataError::BadSeries { number, total });
            }
        }
        Ok(())
    }
}

/// SHA-256 of arbitrary bytes → the 32-byte hash pinned on-chain. The one true hash primitive for
/// `data_hash`, `metadata_hash`, and `license_hash`. Uses `chia_sha2::Sha256` (the same primitive
/// chip35 uses) so the hashes are byte-identical across the ecosystem.
pub fn sha256(bytes: &[u8]) -> Bytes32 {
    let mut h = chia_sha2::Sha256::new();
    h.update(bytes);
    Bytes32::new(h.finalize())
}

/// Validate URI↔hash agreement for one asset: the on-chain `expected` hash MUST equal
/// `sha256(bytes)` of what the URI actually serves. `which` is "data" | "metadata" | "license"
/// for error reporting. This is the footgun-closing check roadmap #36 calls for — the on-chain
/// hash must match the served bytes, or every verifying client rejects the NFT.
pub fn validate_uri_hash(
    which: &'static str,
    bytes: &[u8],
    expected: Bytes32,
) -> Result<(), MetadataError> {
    let computed = sha256(bytes);
    if computed != expected {
        return Err(MetadataError::HashMismatch {
            which,
            computed: format!("{computed}"),
            expected: format!("{expected}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_canonical_format() {
        let md = Chip0007Metadata::new("DIG Punk #1");
        assert_eq!(md.format, CHIP0007_FORMAT);
        assert_eq!(md.name, "DIG Punk #1");
        md.validate_schema().expect("minimal doc is valid");
    }

    #[test]
    fn canonical_json_is_deterministic_and_hash_reproducible() {
        let mut a = Chip0007Metadata::new("Item");
        a.description = Some("hello".into());
        a.attributes = vec![Attribute {
            trait_type: "Background".into(),
            value: "Blue".into(),
        }];
        let mut b = Chip0007Metadata::new("Item");
        b.description = Some("hello".into());
        b.attributes = vec![Attribute {
            trait_type: "Background".into(),
            value: "Blue".into(),
        }];
        assert_eq!(
            a.to_canonical_json().unwrap(),
            b.to_canonical_json().unwrap()
        );
        assert_eq!(
            a.compute_metadata_hash().unwrap(),
            b.compute_metadata_hash().unwrap()
        );
    }

    #[test]
    fn metadata_hash_equals_sha256_of_canonical_json() {
        let md = Chip0007Metadata::new("Item");
        let json = md.to_canonical_json().unwrap();
        assert_eq!(md.compute_metadata_hash().unwrap(), sha256(json.as_bytes()));
    }

    /// The minimal document's canonical JSON must be EXACTLY this byte string — the same one
    /// `chip35_dl_coin` produces. This is the cross-module byte-parity guard (SYSTEM.md): the
    /// compact serde rendering, fixed field order, and omitted optionals are all pinned here. If
    /// this string ever changes, the on-chain `metadata_hash` drifts and NFT verification breaks
    /// ecosystem-wide — change BOTH repos together.
    #[test]
    fn minimal_canonical_json_is_the_pinned_byte_string() {
        let md = Chip0007Metadata::new("Item");
        assert_eq!(
            md.to_canonical_json().unwrap(),
            r#"{"format":"CHIP-0007","name":"Item"}"#,
            "minimal CHIP-0007 JSON must be byte-identical to chip35's"
        );
    }

    /// A fully-populated document pins the full field order + skip rules (the cross-module guard for
    /// the non-trivial case). `sensitive_content:false` and empty vecs/None are omitted.
    #[test]
    fn full_canonical_json_field_order_is_pinned() {
        let mut md = Chip0007Metadata::new("DIG Punk #2");
        md.description = Some("a punk".into());
        md.collection = Some(CollectionRef {
            id: "col1".into(),
            name: "DIG Punks".into(),
            attributes: vec![],
        });
        md.attributes = vec![Attribute {
            trait_type: "Background".into(),
            value: "Blue".into(),
        }];
        md.series_number = Some(2);
        md.series_total = Some(10);
        md.minting_tool = Some("DIG".into());
        assert_eq!(
            md.to_canonical_json().unwrap(),
            r#"{"format":"CHIP-0007","name":"DIG Punk #2","description":"a punk","collection":{"id":"col1","name":"DIG Punks"},"attributes":[{"trait_type":"Background","value":"Blue"}],"series_number":2,"series_total":10,"minting_tool":"DIG"}"#
        );
    }

    #[test]
    fn validate_schema_rejects_bad_format() {
        let mut md = Chip0007Metadata::new("Item");
        md.format = "CHIP-0015".into();
        assert!(matches!(
            md.validate_schema(),
            Err(MetadataError::BadFormat(_))
        ));
    }

    #[test]
    fn validate_schema_rejects_empty_name() {
        let md = Chip0007Metadata::new("   ");
        assert!(matches!(
            md.validate_schema(),
            Err(MetadataError::MissingField("name"))
        ));
    }

    #[test]
    fn validate_schema_rejects_series_overflow() {
        let mut md = Chip0007Metadata::new("Item");
        md.series_number = Some(5);
        md.series_total = Some(3);
        assert!(matches!(
            md.validate_schema(),
            Err(MetadataError::BadSeries {
                number: 5,
                total: 3
            })
        ));
    }

    #[test]
    fn validate_uri_hash_accepts_matching_bytes() {
        let bytes = b"the real media bytes";
        let hash = sha256(bytes);
        validate_uri_hash("data", bytes, hash).expect("matching bytes pass");
    }

    #[test]
    fn validate_uri_hash_rejects_mismatched_bytes() {
        let hash = sha256(b"the real media bytes");
        let err = validate_uri_hash("data", b"different bytes", hash).unwrap_err();
        assert!(matches!(
            err,
            MetadataError::HashMismatch { which: "data", .. }
        ));
    }

    // ---------- #187: collection attributes use `type`, item attributes use `trait_type` ----------

    /// A CHIP-0007-conformant collection attribute (`"type"`, NOT `"trait_type"`) parses. This is
    /// dkackman's exact failure reproduced at the struct level: before the #187 fix, `CollectionRef`
    /// reused `Attribute` (which demands `trait_type`) and rejected this with "missing field
    /// `trait_type`".
    #[test]
    fn collection_attribute_parses_chip0007_type_field() {
        let attr: CollectionAttribute =
            serde_json::from_str(r#"{"type":"icon","value":"https://dig.net/icon.png"}"#)
                .expect("a conformant CHIP-0007 collection attribute must parse");
        assert_eq!(attr.kind, "icon");
        assert_eq!(attr.value, "https://dig.net/icon.png");
    }

    /// Back-compat (§5.1): a collection attribute written with the OLD, non-conformant
    /// `trait_type` field (already-emitted DIG collection.json) still parses via the alias.
    #[test]
    fn collection_attribute_accepts_legacy_trait_type_alias() {
        let attr: CollectionAttribute =
            serde_json::from_str(r#"{"trait_type":"icon","value":"https://dig.net/icon.png"}"#)
                .expect("the legacy trait_type spelling must still parse");
        assert_eq!(attr.kind, "icon");
    }

    /// Going forward, a collection attribute always WRITES `type` (never `trait_type`), so newly
    /// emitted collection.json documents are CHIP-0007 conformant.
    #[test]
    fn collection_attribute_serializes_as_type_not_trait_type() {
        let attr = CollectionAttribute {
            kind: "banner".into(),
            value: "https://dig.net/banner.png".into(),
        };
        let json = serde_json::to_string(&attr).unwrap();
        assert_eq!(
            json,
            r#"{"type":"banner","value":"https://dig.net/banner.png"}"#
        );
    }

    /// NFT **item** attributes are unaffected by #187: they still require `trait_type` and reject a
    /// collection-style `type` field (the two shapes stay distinct).
    #[test]
    fn item_attribute_still_requires_trait_type_not_type() {
        let err = serde_json::from_str::<Attribute>(r#"{"type":"icon","value":"x"}"#).unwrap_err();
        assert!(
            err.to_string().contains("trait_type"),
            "item Attribute must still demand trait_type, got: {err}"
        );
        let ok: Attribute =
            serde_json::from_str(r#"{"trait_type":"Background","value":"Blue"}"#).unwrap();
        assert_eq!(ok.trait_type, "Background");
    }
}
