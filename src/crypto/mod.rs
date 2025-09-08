//! Cryptographic operations for zero-knowledge storage
//!
//! This module provides:
//! - URN transformation using public keys
//! - Data encryption/decryption using URNs
//! - Key derivation functions

pub mod encryption;
pub mod transform;

pub use encryption::{decrypt_data, decrypt_data_with_key, encrypt_data, encrypt_data_with_key};
pub use transform::{transform_urn, PublicKey};

use crate::core::error::Result;
use sha2::{Digest, Sha256};

/// Derive an encryption key from a URN
pub fn derive_key_from_urn(urn: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(urn.as_bytes());
    hasher.finalize().into()
}

/// Derive a storage address from transformed URN
pub fn derive_storage_address(urn: &str, public_key: &PublicKey) -> Result<String> {
    // Transform URN with public key
    let transformed = transform_urn(urn, public_key)?;

    // Hash the result to get a fixed-size address
    let mut hasher = Sha256::new();
    hasher.update(transformed.as_bytes());
    let hash = hasher.finalize();

    // Return as hex string
    Ok(hex::encode(hash))
}
