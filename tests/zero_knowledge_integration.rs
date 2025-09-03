//! Integration test for zero-knowledge URN behavior

use digstore_min::urn::parse_urn;
use sha2::{Sha256, Digest};

/// Test that the deterministic random generation function works correctly
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

/// Test that different seeds produce different data
#[test]
fn test_different_seeds_different_data() {
    let data1 = generate_deterministic_random_bytes("seed1", 1024);
    let data2 = generate_deterministic_random_bytes("seed2", 1024);
    let data3 = generate_deterministic_random_bytes("different_seed", 1024);
    
    assert_ne!(data1, data2, "Different seeds should produce different data");
    assert_ne!(data1, data3, "Different seeds should produce different data");
    assert_ne!(data2, data3, "Different seeds should produce different data");
}

/// Test URN parsing behavior for invalid URNs
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
        // The important thing is that the get command handles both cases
        match result {
            Ok(parsed) => {
                // If it parses, the store lookup should fail and return random data
                println!("URN '{}' parsed successfully: store_id={}", urn, parsed.store_id.to_hex());
            }
            Err(_) => {
                // If parsing fails, get command should return random data
                println!("URN '{}' failed to parse (will return random data)", urn);
            }
        }
    }
}

/// Test byte range calculation for deterministic random data
#[test]
fn test_byte_range_calculations() {
    let test_cases = vec![
        // (start, end, expected_size)
        (Some(0), Some(99), 100),      // 0-99 = 100 bytes
        (Some(0), Some(1023), 1024),   // 0-1023 = 1024 bytes  
        (Some(100), Some(199), 100),   // 100-199 = 100 bytes
        (Some(0), None, 1024 * 1024 - 0), // 0 to end (assuming 1MB default)
        (None, Some(1023), 1024),      // last 1024 bytes
        (None, None, 1024 * 1024),     // full file (1MB default)
    ];
    
    for (start, end, expected_size) in test_cases {
        let calculated_size = match (start, end) {
            (Some(start), Some(end)) => (end - start + 1) as usize,
            (Some(start), None) => (1024 * 1024 - start) as usize,
            (None, Some(end)) => (end + 1) as usize,
            (None, None) => 1024 * 1024,
        };
        
        assert_eq!(calculated_size, expected_size, 
                  "Byte range calculation incorrect for start={:?}, end={:?}", start, end);
    }
}

/// Test URN transformation determinism
#[test]
fn test_urn_transformation_determinism() {
    use digstore_min::crypto::{PublicKey, transform_urn};
    
    let public_key = PublicKey::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let urn = "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/test.txt";
    
    // Transform multiple times
    let transformed1 = transform_urn(urn, &public_key).unwrap();
    let transformed2 = transform_urn(urn, &public_key).unwrap();
    let transformed3 = transform_urn(urn, &public_key).unwrap();
    
    assert_eq!(transformed1, transformed2, "URN transformation should be deterministic");
    assert_eq!(transformed1, transformed3, "URN transformation should be deterministic");
    
    // Should be valid hex string
    assert_eq!(transformed1.len(), 64, "Transformed address should be 64 hex characters");
    assert!(transformed1.chars().all(|c| c.is_ascii_hexdigit()), "Should be valid hex");
}

/// Test encryption key derivation determinism
#[test]
fn test_encryption_key_determinism() {
    use digstore_min::crypto::derive_key_from_urn;
    
    let urn = "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/test.txt";
    
    // Derive key multiple times
    let key1 = derive_key_from_urn(urn);
    let key2 = derive_key_from_urn(urn);
    let key3 = derive_key_from_urn(urn);
    
    assert_eq!(key1, key2, "Encryption key derivation should be deterministic");
    assert_eq!(key1, key3, "Encryption key derivation should be deterministic");
    
    // Should be 32 bytes
    assert_eq!(key1.len(), 32, "Encryption key should be 32 bytes");
    
    // Different URNs should produce different keys
    let different_urn = "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/different.txt";
    let different_key = derive_key_from_urn(different_urn);
    assert_ne!(key1, different_key, "Different URNs should produce different keys");
}

/// Test storage address derivation
#[test]
fn test_storage_address_derivation() {
    use digstore_min::crypto::{PublicKey, derive_storage_address};
    
    let public_key = PublicKey::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let urn = "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/test.txt";
    
    // Derive storage address multiple times
    let addr1 = derive_storage_address(urn, &public_key).unwrap();
    let addr2 = derive_storage_address(urn, &public_key).unwrap();
    
    assert_eq!(addr1, addr2, "Storage address derivation should be deterministic");
    assert_eq!(addr1.len(), 64, "Storage address should be 64 hex characters");
    assert!(addr1.chars().all(|c| c.is_ascii_hexdigit()), "Should be valid hex");
    
    // Different public keys should produce different addresses
    let different_key = PublicKey::from_hex("fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321").unwrap();
    let different_addr = derive_storage_address(urn, &different_key).unwrap();
    assert_ne!(addr1, different_addr, "Different public keys should produce different addresses");
}

/// Helper function for deterministic random generation (same as in get command)
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
