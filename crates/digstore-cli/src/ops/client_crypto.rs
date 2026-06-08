//! Client-side cryptography: key derivation, merkle verify, AES-256-GCM open.
//! All decryption happens HERE (CLIENT-SIDE); the module never decrypts
//! (CONVENTIONS C10).

use digstore_core::{Bytes32, ContentResponse, MerkleProof, SecretSalt, Urn};

use crate::error::CliError;
use crate::ops::store_ops::canonical_resource_urn;

/// Derive the AES-256 key for a URN (§11.3) via the canonical
/// `digstore_crypto::derive_decryption_key` (NO parallel KDF, C10). For private
/// stores the SecretSalt is mixed in (§11.4); a wrong/missing salt yields a wrong
/// key whose GCM tag will not verify. The key is derived from the canonical
/// root-INDEPENDENT resource URN (matching commit-time derivation).
pub fn derive_decryption_key(urn: &Urn, secret_salt: Option<&[u8; 32]>) -> [u8; 32] {
    let canonical = canonical_resource_urn(urn.store_id, urn.resource_key.as_deref().unwrap_or(""));
    let salt = secret_salt.map(|s| SecretSalt(*s));
    digstore_crypto::derive_decryption_key(&canonical.canonical(), salt.as_ref())
}

/// Verify (§9.3) that `bytes` is the proof's leaf, the path resolves to
/// `proof.root`, and `proof.root == trusted_root`. leaf=SHA-256(bytes);
/// node=SHA-256(left||right).
pub fn verify_chunk_inclusion(
    bytes: &[u8],
    proof: &MerkleProof,
    trusted_root: &Bytes32,
) -> Result<(), CliError> {
    let computed_leaf = digstore_crypto::sha256(bytes);
    if computed_leaf != proof.leaf {
        return Err(CliError::VerificationFailed(
            "content does not match proof leaf (tampered chunk)".into(),
        ));
    }
    if !proof.verify() {
        return Err(CliError::VerificationFailed(
            "merkle path does not resolve to declared root".into(),
        ));
    }
    if &proof.root != trusted_root {
        return Err(CliError::VerificationFailed(
            "merkle root does not match trusted root".into(),
        ));
    }
    Ok(())
}

/// Full client pipeline (§9.3 + §11): verify the served bytes' merkle inclusion
/// against the trusted root, then split the length-framed chunk ciphertexts and
/// AES-256-GCM open each (tag verified) under the resource's URN key, finally
/// concatenating the plaintext in order.
pub fn decrypt_and_verify(
    resp: &ContentResponse,
    urn: &Urn,
    secret_salt: Option<&[u8; 32]>,
    trusted_root: &Bytes32,
) -> Result<Vec<u8>, CliError> {
    // 1) integrity: the served bytes are committed under the trusted root.
    verify_chunk_inclusion(&resp.ciphertext, &resp.merkle_proof, trusted_root)?;

    // 2) confidentiality: split frames and open each chunk.
    let key = derive_decryption_key(urn, secret_salt);
    let mut plaintext = Vec::new();
    let buf = &resp.ciphertext;
    let mut p = 0usize;
    while p < buf.len() {
        if p + 4 > buf.len() {
            return Err(CliError::VerificationFailed("truncated chunk frame".into()));
        }
        let len = u32::from_be_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]]) as usize;
        p += 4;
        if p + len > buf.len() {
            return Err(CliError::VerificationFailed("chunk frame out of bounds".into()));
        }
        let ct = &buf[p..p + len];
        p += len;
        let pt = digstore_crypto::decrypt_chunk(&key, ct).map_err(|_| {
            CliError::VerificationFailed(
                "AES-256-GCM tag verification failed (wrong key/salt or tampered ciphertext)".into(),
            )
        })?;
        plaintext.extend_from_slice(&pt);
    }
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{MerkleTree, ProofStep};

    fn urn() -> Urn {
        Urn {
            chain: "chia".into(),
            store_id: Bytes32([7u8; 32]),
            root_hash: Some(Bytes32([9u8; 32])),
            resource_key: Some("readme".into()),
        }
    }

    /// Build a framed, single-chunk resource ciphertext with a one-leaf tree.
    fn framed_single(key: &[u8; 32], pt: &[u8]) -> (Vec<u8>, Bytes32) {
        let ct = digstore_crypto::encrypt_chunk(key, pt);
        let mut framed = Vec::new();
        framed.extend_from_slice(&(ct.len() as u32).to_be_bytes());
        framed.extend_from_slice(&ct);
        let leaf = digstore_crypto::sha256(&framed);
        (framed, leaf)
    }

    #[test]
    fn key_is_deterministic_and_32_bytes() {
        assert_eq!(derive_decryption_key(&urn(), None), derive_decryption_key(&urn(), None));
        assert_eq!(derive_decryption_key(&urn(), None).len(), 32);
    }

    #[test]
    fn private_salt_changes_the_key() {
        let public = derive_decryption_key(&urn(), None);
        let private = derive_decryption_key(&urn(), Some(&[3u8; 32]));
        assert_ne!(public, private);
    }

    #[test]
    fn single_chunk_round_trips() {
        let urn = urn();
        let key = derive_decryption_key(&urn, None);
        let pt = b"the quick brown fox".to_vec();
        let (framed, leaf) = framed_single(&key, &pt);
        let resp = ContentResponse {
            ciphertext: framed,
            merkle_proof: MerkleProof {
                leaf,
                path: vec![],
                root: leaf,
            },
            roothash: leaf,
        };
        assert_eq!(decrypt_and_verify(&resp, &urn, None, &leaf).unwrap(), pt);
    }

    #[test]
    fn wrong_trusted_root_fails_at_merkle_gate() {
        let urn = urn();
        let key = derive_decryption_key(&urn, None);
        let (framed, leaf) = framed_single(&key, b"data");
        let resp = ContentResponse {
            ciphertext: framed,
            merkle_proof: MerkleProof {
                leaf,
                path: vec![],
                root: leaf,
            },
            roothash: leaf,
        };
        let err = decrypt_and_verify(&resp, &urn, None, &Bytes32([0xFF; 32])).unwrap_err();
        assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("trusted root")));
    }

    #[test]
    fn tampered_ciphertext_fails_at_merkle_gate_first() {
        let urn = urn();
        let key = derive_decryption_key(&urn, None);
        let (mut framed, leaf) = framed_single(&key, b"data");
        framed[5] ^= 0xFF; // mutate ciphertext -> leaf mismatch
        let resp = ContentResponse {
            ciphertext: framed,
            merkle_proof: MerkleProof {
                leaf,
                path: vec![],
                root: leaf,
            },
            roothash: leaf,
        };
        let err = decrypt_and_verify(&resp, &urn, None, &leaf).unwrap_err();
        assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("tampered chunk")));
    }

    #[test]
    fn decoy_fabricated_root_fails_at_merkle_gate() {
        let urn = urn();
        let key = derive_decryption_key(&urn, None);
        let (framed, leaf) = framed_single(&key, b"decoy");
        let trusted = Bytes32([0x11; 32]);
        let resp = ContentResponse {
            ciphertext: framed,
            merkle_proof: MerkleProof {
                leaf,
                path: vec![],
                root: leaf, // fabricated
            },
            roothash: leaf,
        };
        let err = decrypt_and_verify(&resp, &urn, None, &trusted).unwrap_err();
        assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("trusted root")));
    }

    #[test]
    fn two_leaf_path_verifies() {
        let urn = urn();
        let key = derive_decryption_key(&urn, None);
        let (framed, leaf0) = framed_single(&key, b"resource-zero");
        let sibling = Bytes32([0x55; 32]);
        let tree = MerkleTree::from_leaves(vec![leaf0, sibling]);
        let root = tree.root();
        let proof = MerkleProof {
            leaf: leaf0,
            path: vec![ProofStep {
                hash: sibling,
                is_left: false,
            }],
            root,
        };
        let resp = ContentResponse {
            ciphertext: framed,
            merkle_proof: proof,
            roothash: root,
        };
        assert_eq!(decrypt_and_verify(&resp, &urn, None, &root).unwrap(), b"resource-zero");
    }
}
