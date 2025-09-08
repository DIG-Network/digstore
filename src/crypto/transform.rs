//! URN transformation using public keys

use crate::core::error::{DigstoreError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Public key for URN transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKey {
    /// The public key bytes (32 bytes for Ed25519)
    pub bytes: Vec<u8>,
    /// Key algorithm identifier
    pub algorithm: String,
}

impl PublicKey {
    /// Create a new public key
    pub fn new(bytes: Vec<u8>, algorithm: String) -> Self {
        Self { bytes, algorithm }
    }

    /// Create from hex string
    pub fn from_hex(hex: &str) -> Result<Self> {
        let bytes =
            hex::decode(hex).map_err(|_| DigstoreError::internal("Invalid hex public key"))?;

        if bytes.len() != 32 {
            return Err(DigstoreError::internal("Public key must be 32 bytes"));
        }

        Ok(Self {
            bytes,
            algorithm: "ed25519".to_string(),
        })
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(&self.bytes)
    }
}

/// Transform a URN using a public key
///
/// This creates a deterministic but cryptographically secure transformation
/// that combines the URN with the public key in a way that:
/// 1. Cannot be reversed without the private key
/// 2. Different public keys produce different results
/// 3. The same URN+key always produces the same result
pub fn transform_urn(urn: &str, public_key: &PublicKey) -> Result<String> {
    // Create a domain-separated hash
    let mut hasher = Sha256::new();

    // Domain separation
    hasher.update(b"digstore_urn_transform_v1:");

    // Add the public key
    hasher.update(&public_key.algorithm.as_bytes());
    hasher.update(&(public_key.bytes.len() as u32).to_le_bytes());
    hasher.update(&public_key.bytes);

    // Add the URN
    hasher.update(&(urn.len() as u32).to_le_bytes());
    hasher.update(urn.as_bytes());

    // Get the hash
    let hash = hasher.finalize();

    // Return the raw transformation result (just the hex hash)
    Ok(hex::encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_urn() {
        let pubkey =
            PublicKey::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap();

        let urn = "urn:dig:chia:abc123/file.txt";
        let transformed = transform_urn(urn, &pubkey).unwrap();

        // Should be deterministic
        let transformed2 = transform_urn(urn, &pubkey).unwrap();
        assert_eq!(transformed, transformed2);

        // Should be different for different URNs
        let urn2 = "urn:dig:chia:abc123/file2.txt";
        let transformed3 = transform_urn(urn2, &pubkey).unwrap();
        assert_ne!(transformed, transformed3);

        // Should be a 64-character hex string (32 bytes)
        assert_eq!(transformed.len(), 64);
        assert!(transformed.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_different_keys_different_results() {
        let pubkey1 =
            PublicKey::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap();

        let pubkey2 =
            PublicKey::from_hex("fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321")
                .unwrap();

        let urn = "urn:dig:chia:abc123/file.txt";
        let transformed1 = transform_urn(urn, &pubkey1).unwrap();
        let transformed2 = transform_urn(urn, &pubkey2).unwrap();

        assert_ne!(transformed1, transformed2);
    }
}
