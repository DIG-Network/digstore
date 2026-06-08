use digstore_core::Bytes32;
use sha2::{Digest, Sha256};

/// SHA-256 over `data`, returned as the canonical `Bytes32` newtype.
pub fn sha256(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    Bytes32(arr)
}
