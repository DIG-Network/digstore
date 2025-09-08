//! Unit tests for archive size proof core functionality
//!
//! These tests validate the internal proof structures, compression, and cryptographic operations.

use digstore_min::core::types::{Hash, StoreId};
use digstore_min::proofs::size_proof::{ArchiveSizeProof, CompressedSizeProof, IntegrityProofs};
use std::collections::HashMap;

/// Test the basic proof structure creation and serialization
#[test]
fn test_archive_size_proof_creation() {
    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap();

    let proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 5,
        calculated_total_size: 1024 * 1024, // 1MB
        layer_sizes: vec![100, 200, 300, 400, 500],
        layer_size_tree_root: Hash::zero(),
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::zero(),
            layer_index_hash: Hash::zero(),
            root_hash_verification: Hash::zero(),
            first_layer_content_hash: Hash::zero(),
        },
    };

    // Test basic properties
    assert_eq!(proof.store_id, store_id);
    assert_eq!(proof.root_hash, root_hash);
    assert_eq!(proof.verified_layer_count, 5);
    assert_eq!(proof.calculated_total_size, 1024 * 1024);
    assert_eq!(proof.layer_sizes.len(), 5);
    assert_eq!(proof.layer_sizes.iter().sum::<u64>(), 1500);
}

/// Test proof compression and decompression
#[test]
fn test_proof_compression() -> anyhow::Result<()> {
    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")?;
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?;

    let original_proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 10,
        calculated_total_size: 5 * 1024 * 1024, // 5MB
        layer_sizes: vec![512000, 1024000, 768000, 2048000, 1048576],
        layer_size_tree_root: Hash::from_hex(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )?,
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::from_hex(
                "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            )?,
            layer_index_hash: Hash::from_hex(
                "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            )?,
            root_hash_verification: root_hash,
            first_layer_content_hash: Hash::from_hex(
                "1111111111111111111111111111111111111111111111111111111111111111",
            )?,
        },
    };

    // Test compression
    let compressed = original_proof.compress()?;

    // Verify compression properties
    assert_eq!(compressed.store_id, store_id.as_bytes());
    assert_eq!(compressed.root_hash, root_hash.as_bytes());
    assert_eq!(compressed.layer_count, 10);
    assert_eq!(compressed.total_size, 5 * 1024 * 1024);

    // Test decompression
    let decompressed = compressed.to_archive_proof()?;

    // Verify decompression accuracy
    assert_eq!(decompressed.store_id, original_proof.store_id);
    assert_eq!(decompressed.root_hash, original_proof.root_hash);
    assert_eq!(
        decompressed.verified_layer_count,
        original_proof.verified_layer_count
    );
    assert_eq!(
        decompressed.calculated_total_size,
        original_proof.calculated_total_size
    );

    Ok(())
}

/// Test hex encoding and decoding
#[test]
fn test_hex_encoding_decoding() -> anyhow::Result<()> {
    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")?;
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?;

    let proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 3,
        calculated_total_size: 2048,
        layer_sizes: vec![512, 1024, 512],
        layer_size_tree_root: Hash::from_hex(
            "2222222222222222222222222222222222222222222222222222222222222222",
        )?,
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::from_hex(
                "3333333333333333333333333333333333333333333333333333333333333333",
            )?,
            layer_index_hash: Hash::from_hex(
                "4444444444444444444444444444444444444444444444444444444444444444",
            )?,
            root_hash_verification: root_hash,
            first_layer_content_hash: Hash::from_hex(
                "5555555555555555555555555555555555555555555555555555555555555555",
            )?,
        },
    };

    // Compress and encode to hex
    let compressed = proof.compress()?;
    let hex_encoded = compressed.to_hex_string()?;

    // Verify hex properties
    assert!(!hex_encoded.is_empty());
    assert!(
        hex_encoded.chars().all(|c| c.is_ascii_hexdigit()),
        "Hex string should only contain hex digits"
    );
    assert!(
        hex_encoded.len() % 2 == 0,
        "Hex string should have even length"
    );

    // Test decoding
    let decoded_compressed = CompressedSizeProof::from_hex_string(&hex_encoded)?;
    let decoded_proof = decoded_compressed.to_archive_proof()?;

    // Verify round-trip accuracy
    assert_eq!(decoded_proof.store_id, proof.store_id);
    assert_eq!(decoded_proof.root_hash, proof.root_hash);
    assert_eq!(
        decoded_proof.verified_layer_count,
        proof.verified_layer_count
    );
    assert_eq!(
        decoded_proof.calculated_total_size,
        proof.calculated_total_size
    );

    Ok(())
}

/// Test compression ratio effectiveness
#[test]
fn test_compression_effectiveness() -> anyhow::Result<()> {
    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")?;
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?;

    // Create a large proof with many layers
    let layer_sizes: Vec<u64> = (0..100).map(|i| (i + 1) * 1024).collect();
    let total_size: u64 = layer_sizes.iter().sum();

    let proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 100,
        calculated_total_size: total_size,
        layer_sizes,
        layer_size_tree_root: Hash::from_hex(
            "6666666666666666666666666666666666666666666666666666666666666666",
        )?,
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::from_hex(
                "7777777777777777777777777777777777777777777777777777777777777777",
            )?,
            layer_index_hash: Hash::from_hex(
                "8888888888888888888888888888888888888888888888888888888888888888",
            )?,
            root_hash_verification: root_hash,
            first_layer_content_hash: Hash::from_hex(
                "9999999999999999999999999999999999999999999999999999999999999999",
            )?,
        },
    };

    // Test JSON serialization size (uncompressed baseline)
    let json_size = serde_json::to_string(&proof)?.len();

    // Test compressed size
    let compressed = proof.compress()?;
    let hex_encoded = compressed.to_hex_string()?;
    let compressed_size = hex_encoded.len();

    // Compression should be significantly smaller
    let compression_ratio = compressed_size as f64 / json_size as f64;
    assert!(
        compression_ratio < 0.8,
        "Compression ratio should be < 80%, got {:.2}% ({} vs {} bytes)",
        compression_ratio * 100.0,
        compressed_size,
        json_size
    );

    // But should still be substantial (not over-compressed)
    assert!(
        compressed_size > 200,
        "Compressed proof too small, might be missing data: {} bytes",
        compressed_size
    );

    Ok(())
}

/// Test invalid input handling
#[test]
fn test_invalid_inputs() {
    // Test invalid hex strings
    assert!(CompressedSizeProof::from_hex_string("invalid_hex").is_err());
    assert!(CompressedSizeProof::from_hex_string("abcdefg").is_err()); // odd length
    assert!(CompressedSizeProof::from_hex_string("").is_err()); // empty

    // Test invalid hash formats
    assert!(Hash::from_hex("invalid_length").is_err());
    assert!(Hash::from_hex("").is_err());
    assert!(Hash::from_hex("not_hex_chars_!@#$%^&*()").is_err());
}

/// Test edge cases with empty and minimal data
#[test]
fn test_edge_cases() -> anyhow::Result<()> {
    let store_id = Hash::zero();
    let root_hash = Hash::zero();

    // Test with minimal data
    let minimal_proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 1,
        calculated_total_size: 0,
        layer_sizes: vec![],
        layer_size_tree_root: Hash::zero(),
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::zero(),
            layer_index_hash: Hash::zero(),
            root_hash_verification: Hash::zero(),
            first_layer_content_hash: Hash::zero(),
        },
    };

    // Should still compress and decompress successfully
    let compressed = minimal_proof.compress()?;
    let hex_encoded = compressed.to_hex_string()?;
    let decoded = CompressedSizeProof::from_hex_string(&hex_encoded)?;
    let decompressed = decoded.to_archive_proof()?;

    assert_eq!(decompressed.store_id, minimal_proof.store_id);
    assert_eq!(decompressed.calculated_total_size, 0);
    assert_eq!(decompressed.layer_sizes.len(), 0);

    Ok(())
}

/// Test large archive handling
#[test]
fn test_large_archive_proof() -> anyhow::Result<()> {
    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")?;
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?;

    // Test with very large archive (10GB)
    let large_size = 10u64 * 1024 * 1024 * 1024; // 10GB
    let layer_count = 1000;
    let layer_sizes: Vec<u64> = (0..layer_count).map(|_| large_size / layer_count).collect();

    let large_proof = ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: layer_count as u32,
        calculated_total_size: large_size,
        layer_sizes,
        layer_size_tree_root: Hash::from_hex(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )?,
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::from_hex(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )?,
            layer_index_hash: Hash::from_hex(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            )?,
            root_hash_verification: root_hash,
            first_layer_content_hash: Hash::from_hex(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            )?,
        },
    };

    // Should handle large numbers correctly
    let compressed = large_proof.compress()?;
    let hex_encoded = compressed.to_hex_string()?;
    let decoded = CompressedSizeProof::from_hex_string(&hex_encoded)?;
    let decompressed = decoded.to_archive_proof()?;

    assert_eq!(decompressed.calculated_total_size, large_size);
    assert_eq!(decompressed.verified_layer_count, layer_count as u32);

    // Proof should still be reasonably sized even for large archives
    assert!(
        hex_encoded.len() < 10000,
        "Proof for 10GB archive should still be < 10KB, got {} bytes",
        hex_encoded.len()
    );

    Ok(())
}

/// Test concurrent access and thread safety
#[cfg(feature = "std")]
#[test]
fn test_thread_safety() -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::thread;

    let store_id =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")?;
    let root_hash =
        Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?;

    let proof = Arc::new(ArchiveSizeProof {
        store_id,
        root_hash,
        verified_layer_count: 5,
        calculated_total_size: 1024,
        layer_sizes: vec![200, 200, 200, 200, 224],
        layer_size_tree_root: Hash::zero(),
        integrity_proofs: IntegrityProofs {
            archive_header_hash: Hash::zero(),
            layer_index_hash: Hash::zero(),
            root_hash_verification: Hash::zero(),
            first_layer_content_hash: Hash::zero(),
        },
    });

    let mut handles = vec![];

    // Spawn multiple threads to compress the same proof
    for i in 0..10 {
        let proof_clone = Arc::clone(&proof);
        let handle = thread::spawn(move || -> anyhow::Result<String> {
            let compressed = proof_clone.compress()?;
            let hex = compressed.to_hex_string()?;
            Ok(hex)
        });
        handles.push(handle);
    }

    // All threads should produce identical results
    let mut results = vec![];
    for handle in handles {
        results.push(handle.join().unwrap()?);
    }

    // All results should be identical
    let first_result = &results[0];
    for result in &results[1..] {
        assert_eq!(
            result, first_result,
            "Thread safety violation: different results"
        );
    }

    Ok(())
}
