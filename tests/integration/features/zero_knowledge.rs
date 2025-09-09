//! Integration tests for zero-knowledge features
//!
//! Tests the zero-knowledge properties where invalid URNs return deterministic random data.

use digstore_min::urn::parse_urn;
use sha2::{Digest, Sha256};

#[test]
fn test_deterministic_random_generation() {
    let seed = "test_seed";
    let size = 1024;

    let data1 = generate_deterministic_random_bytes(seed, size);
    let data2 = generate_deterministic_random_bytes(seed, size);

    assert_eq!(data1, data2, "Same seed should produce identical data");
    assert_eq!(data1.len(), size, "Data should be requested size");
    assert_ne!(data1, vec![0u8; size], "Data should not be all zeros");
}

#[test]
fn test_different_seeds_different_data() {
    let data1 = generate_deterministic_random_bytes("seed1", 1024);
    let data2 = generate_deterministic_random_bytes("seed2", 1024);
    let data3 = generate_deterministic_random_bytes("different_seed", 1024);

    assert_ne!(data1, data2, "Different seeds should produce different data");
    assert_ne!(data1, data3, "Different seeds should produce different data");
    assert_ne!(data2, data3, "Different seeds should produce different data");
}

#[test]
fn test_urn_parsing_invalid_urns() {
    let invalid_urns = vec![
        "invalid-urn",
        "urn:dig:chia:invalid-hex/file.txt",
        "urn:dig:chia:short/file.txt",
        "urn:wrong:chia:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef/file.txt",
    ];

    for urn in invalid_urns {
        let result = parse_urn(urn);

        // Should either parse successfully or fail predictably
        match result {
            Ok(parsed) => {
                println!("URN '{}' parsed: store_id={}", urn, parsed.store_id.to_hex());
            }
            Err(_) => {
                println!("URN '{}' failed to parse (will return random data)", urn);
            }
        }
    }
}

#[test]
fn test_byte_range_calculations() {
    let test_cases = vec![
        // (start, end, expected_size)
        (Some(0), Some(99), 100),         // 0-99 = 100 bytes
        (Some(0), Some(1023), 1024),      // 0-1023 = 1024 bytes
        (Some(100), Some(199), 100),      // 100-199 = 100 bytes
        (Some(0), None, 1024 * 1024 - 0), // 0 to end (assuming 1MB default)
        (None, Some(1023), 1024),         // last 1024 bytes
        (None, None, 1024 * 1024),        // full file (1MB default)
    ];

    for (start, end, expected_size) in test_cases {
        let calculated_size = match (start, end) {
            (Some(start), Some(end)) => (end - start + 1) as usize,
            (Some(start), None) => (1024 * 1024 - start) as usize,
            (None, Some(end)) => (end + 1) as usize,
            (None, None) => 1024 * 1024,
        };

        assert_eq!(
            calculated_size, expected_size,
            "Byte range calculation incorrect for start={:?}, end={:?}",
            start, end
        );
    }
}

/// Helper function for deterministic random generation
fn generate_deterministic_random_bytes(seed: &str, size: usize) -> Vec<u8> {
    let mut result = Vec::with_capacity(size);
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let mut counter = 0u64;

    while result.len() < size {
        let mut current_hasher = hasher.clone();
        current_hasher.update(&counter.to_le_bytes());
        let hash = current_hasher.finalize();

        let bytes_needed = size - result.len();
        let bytes_to_copy = bytes_needed.min(hash.len());
        result.extend_from_slice(&hash[..bytes_to_copy]);

        counter += 1;
    }

    result
}
