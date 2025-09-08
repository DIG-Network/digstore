//! Basic integration tests for Digstore Min
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use digstore_min::core::{hash::*, types::*};

#[test]
fn test_hash_functionality() {
    let data = b"Hello, Digstore Min!";
    let hash1 = sha256(data);
    let hash2 = sha256(data);

    // Hashing should be deterministic
    assert_eq!(hash1, hash2);

    // Different data should produce different hashes
    let hash3 = sha256(b"Different data");
    assert_ne!(hash1, hash3);

    // Test hex conversion
    let hex_str = hash1.to_hex();
    let parsed_hash = Hash::from_hex(&hex_str).unwrap();
    assert_eq!(hash1, parsed_hash);
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
fn test_layer_type_conversions() {
    let layer_type = LayerType::Full;
    let byte_val = layer_type.to_byte();
    let parsed_type = LayerType::from_byte(byte_val).unwrap();
    assert_eq!(layer_type, parsed_type);

    // Test all variants
    assert_eq!(LayerType::Header.to_byte(), 0x00);
    assert_eq!(LayerType::Full.to_byte(), 0x01);
    assert_eq!(LayerType::Delta.to_byte(), 0x02);

    assert_eq!(LayerType::from_byte(0x00).unwrap(), LayerType::Header);
    assert_eq!(LayerType::from_byte(0x01).unwrap(), LayerType::Full);
    assert_eq!(LayerType::from_byte(0x02).unwrap(), LayerType::Delta);
    assert!(LayerType::from_byte(0xFF).is_none());
}

#[test]
fn test_layer_header_creation() {
    let parent_hash = Hash::zero();
    let header = LayerHeader::new(LayerType::Full, 1, parent_hash);

    assert_eq!(header.magic, LayerHeader::MAGIC);
    assert_eq!(header.version, LayerHeader::VERSION);
    assert_eq!(header.get_layer_type().unwrap(), LayerType::Full);
    assert_eq!(header.layer_number, 1);
    assert_eq!(header.get_parent_hash(), parent_hash);
    assert!(header.is_valid());
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
fn test_hash_zero() {
    let zero_hash = Hash::zero();
    assert_eq!(zero_hash.to_hex(), "0".repeat(64));

    let non_zero = sha256(b"test");
    assert_ne!(non_zero, zero_hash);
}
