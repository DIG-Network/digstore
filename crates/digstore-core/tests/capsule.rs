//! Tests for the canonical `Capsule` identity = `(storeId, rootHash)`.
//!
//! A capsule is one immutable store generation; canonical string is
//! `storeId:rootHash` (lowercase hex : lowercase hex). See SYSTEM.md → capsule.

use digstore_core::codec::{Decode, Encode};
use digstore_core::urn::Urn;
use digstore_core::{Bytes32, Capsule};

fn store_id() -> Bytes32 {
    Bytes32([0x11; 32])
}
fn root_hash() -> Bytes32 {
    Bytes32([0x22; 32])
}

#[test]
fn canonical_is_store_id_colon_root_hash() {
    let cap = Capsule {
        store_id: store_id(),
        root_hash: root_hash(),
    };
    assert_eq!(
        cap.canonical(),
        format!("{}:{}", store_id().to_hex(), root_hash().to_hex())
    );
}

#[test]
fn canonical_roundtrips_from_canonical() {
    let cap = Capsule {
        store_id: store_id(),
        root_hash: root_hash(),
    };
    let s = cap.canonical();
    let back = Capsule::from_canonical(&s).expect("parse canonical");
    assert_eq!(back, cap);
}

#[test]
fn display_matches_canonical() {
    let cap = Capsule {
        store_id: store_id(),
        root_hash: root_hash(),
    };
    assert_eq!(format!("{cap}"), cap.canonical());
}

#[test]
fn from_canonical_rejects_missing_colon() {
    // A single segment (no ':') is not a capsule.
    assert!(Capsule::from_canonical(&store_id().to_hex()).is_err());
}

#[test]
fn from_canonical_rejects_three_segments() {
    let s = format!(
        "{}:{}:{}",
        store_id().to_hex(),
        root_hash().to_hex(),
        root_hash().to_hex()
    );
    assert!(Capsule::from_canonical(&s).is_err());
}

#[test]
fn from_canonical_rejects_short_hex() {
    let s = format!("{}:{}", "11", root_hash().to_hex());
    assert!(Capsule::from_canonical(&s).is_err());
}

#[test]
fn from_canonical_rejects_long_hex() {
    let s = format!("{}:{}", store_id().to_hex(), root_hash().to_hex() + "00");
    assert!(Capsule::from_canonical(&s).is_err());
}

#[test]
fn from_canonical_rejects_non_hex() {
    let s = format!("{}:{}", "zz".repeat(32), root_hash().to_hex());
    assert!(Capsule::from_canonical(&s).is_err());
}

#[test]
fn from_canonical_rejects_empty_segment() {
    // Trailing colon → empty second segment.
    let s = format!("{}:", store_id().to_hex());
    assert!(Capsule::from_canonical(&s).is_err());
    // Leading colon → empty first segment.
    let s = format!(":{}", root_hash().to_hex());
    assert!(Capsule::from_canonical(&s).is_err());
}

#[test]
fn capsule_codec_roundtrips() {
    let cap = Capsule {
        store_id: store_id(),
        root_hash: root_hash(),
    };
    let bytes = cap.to_bytes();
    let back = Capsule::from_bytes(&bytes).expect("decode");
    assert_eq!(back, cap);
}

#[test]
fn capsule_codec_is_two_raw_bytes32() {
    // Capsule encoding mirrors Urn's field-by-field codec: two raw Bytes32, no
    // length prefix → exactly 64 bytes.
    let cap = Capsule {
        store_id: store_id(),
        root_hash: root_hash(),
    };
    let bytes = cap.to_bytes();
    assert_eq!(bytes.len(), 64);
    assert_eq!(&bytes[0..32], &[0x11; 32]);
    assert_eq!(&bytes[32..64], &[0x22; 32]);
}

// --- URN → Capsule bridge ---

#[test]
fn urn_with_root_yields_capsule() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let urn = Urn::parse(&format!("urn:dig:mainnet:{sid}:{rh}/a/b")).unwrap();
    let cap = urn.as_capsule().expect("urn with root has a capsule");
    // The capsule's canonical string equals the `storeId:rootHash` portion of the
    // URN's canonical string.
    assert_eq!(cap.canonical(), format!("{sid}:{rh}"));
    assert_eq!(cap.store_id, store_id());
    assert_eq!(cap.root_hash, root_hash());
}

#[test]
fn rootless_urn_yields_no_capsule() {
    let sid = store_id().to_hex();
    let urn = Urn::parse(&format!("urn:dig:mainnet:{sid}/index.html")).unwrap();
    assert!(urn.as_capsule().is_none());
}
