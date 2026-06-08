//! SHA-256 helper (no_std via the `sha2` crate).

use crate::bytes::Bytes32;
use sha2::{Digest, Sha256};

/// Compute SHA-256 over `data` and wrap it in a `Bytes32`.
pub fn sha256(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    Bytes32(arr)
}
