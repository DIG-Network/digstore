//! URN parsing and handling tests
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use digstore_min::{core::types::Hash, urn::*};
use std::path::PathBuf;

#[test]
fn test_parse_simple_urn() {
    let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2";
    let urn = parse_urn(urn_str).unwrap();

    assert_eq!(
        urn.store_id.to_hex(),
        "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
    );
    assert!(urn.root_hash.is_none());
    assert!(urn.resource_path.is_none());
    assert!(urn.byte_range.is_none());
}

#[test]
fn test_parse_urn_with_path() {
    let urn_str =
        "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/src/main.rs";
    let urn = parse_urn(urn_str).unwrap();

    assert_eq!(urn.resource_path.unwrap(), PathBuf::from("src/main.rs"));
}

#[test]
fn test_parse_urn_with_root_hash() {
    let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let urn = parse_urn(urn_str).unwrap();

    assert!(urn.root_hash.is_some());
    assert_eq!(
        urn.root_hash.unwrap().to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn test_parse_urn_with_byte_range() {
    let test_cases = vec![
        ("urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/file.txt#bytes=0-1023", Some(0), Some(1023)),
        ("urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/file.txt#bytes=1024-", Some(1024), None),
        ("urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/file.txt#bytes=-1024", None, Some(1024)),
    ];

    for (urn_str, expected_start, expected_end) in test_cases {
        let urn = parse_urn(urn_str).unwrap();
        let byte_range = urn.byte_range.unwrap();
        assert_eq!(byte_range.start, expected_start);
        assert_eq!(byte_range.end, expected_end);
    }
}

#[test]
fn test_parse_complex_urn() {
    let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/src/main.rs#bytes=100-200";
    let urn = parse_urn(urn_str).unwrap();

    assert_eq!(
        urn.store_id.to_hex(),
        "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
    );
    assert_eq!(
        urn.root_hash.unwrap().to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(urn.resource_path.unwrap(), PathBuf::from("src/main.rs"));

    let byte_range = urn.byte_range.unwrap();
    assert_eq!(byte_range.start, Some(100));
    assert_eq!(byte_range.end, Some(200));
}

#[test]
fn test_parse_invalid_urn() {
    let invalid_urns = vec![
        "invalid:urn:format",
        "urn:dig:chia:",
        "urn:dig:chia:invalid_hex",
        "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2#bytes=invalid",
        "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2#bytes=100-50", // start > end
    ];

    for urn_str in invalid_urns {
        assert!(
            parse_urn(urn_str).is_err(),
            "Should fail to parse: {}",
            urn_str
        );
    }
}

#[test]
fn test_byte_range_creation() {
    let range1 = ByteRange::new(Some(0), Some(1023));
    assert_eq!(range1.start, Some(0));
    assert_eq!(range1.end, Some(1023));

    let range2 = ByteRange::from_start(1024);
    assert_eq!(range2.start, Some(1024));
    assert_eq!(range2.end, None);

    let range3 = ByteRange::last_bytes(512);
    assert_eq!(range3.start, None);
    assert_eq!(range3.end, Some(512));
}

#[test]
fn test_byte_range_to_string() {
    let test_cases = vec![
        (ByteRange::new(Some(0), Some(1023)), "#bytes=0-1023"),
        (ByteRange::from_start(1024), "#bytes=1024-"),
        (ByteRange::last_bytes(512), "#bytes=-512"),
        (ByteRange::new(None, None), ""),
    ];

    for (range, expected) in test_cases {
        assert_eq!(range.to_string(), expected);
    }
}

#[test]
fn test_urn_roundtrip() {
    let original_urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/src/main.rs#bytes=100-200";
    let urn = parse_urn(original_urn_str).unwrap();
    let reconstructed = urn.to_string();

    // Parse the reconstructed URN to ensure it's valid
    let urn2 = parse_urn(&reconstructed).unwrap();
    assert_eq!(urn.store_id, urn2.store_id);
    assert_eq!(urn.root_hash, urn2.root_hash);
    assert_eq!(urn.resource_path, urn2.resource_path);
}
