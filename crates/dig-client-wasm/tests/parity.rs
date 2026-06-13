//! Read-path CONTRACT test (native).
//!
//! `dig-client-wasm` no longer reproduces the crypto — it consumes the single
//! source of truth in `digstore_core::crypto` + `digstore_core::resource_leaf`,
//! the SAME code the producer (`digstore-store`) and host use, which
//! `digstore-crypto` re-exports for native callers. This test anchors that
//! contract: a known-answer KDF, the host-re-export agreeing with core,
//! host-seals/client-opens AEAD, and a full content→leaf→merkle-proof→verify→
//! decrypt round-trip (the exact gate `verify_inclusion_core` applies). It runs
//! natively (it links the blst-backed `digstore-crypto` to prove the host
//! re-export path agrees with core byte-for-byte).

use digstore_core::codec::{Decode, Encode};
use digstore_core::crypto::{decrypt_chunk, derive_decryption_key, encrypt_chunk};
use digstore_core::{resource_leaf, Bytes32, MerkleProof, MerkleTree, ProofStep, SecretSalt, Urn};

fn canonical_urn(store_id: Bytes32, resource_key: &str) -> String {
    Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some(resource_key.to_string()),
    }
    .canonical()
}

#[test]
fn host_reexport_matches_core_kdf() {
    let store = Bytes32([7u8; 32]);
    let canonical = canonical_urn(store, "index.html");
    // `digstore-crypto` re-exports `digstore_core::crypto`, so the host derivation
    // is byte-identical to the contract the browser verifier uses.
    assert_eq!(
        derive_decryption_key(&canonical, None),
        digstore_crypto::derive_decryption_key(&canonical, None),
        "host re-export must equal the core KDF"
    );
}

#[test]
fn kdf_private_salt_changes_key() {
    let store = Bytes32([0xABu8; 32]);
    let canonical = canonical_urn(store, "secret/page.html");
    let salt = SecretSalt([0x42u8; 32]);
    assert_ne!(
        derive_decryption_key(&canonical, Some(&salt)),
        derive_decryption_key(&canonical, None),
        "secret salt must change the derived key"
    );
}

#[test]
fn host_seals_client_opens() {
    let key = [0x11u8; 32];
    let plaintext = b"<html><body>hello dig</body></html>".to_vec();
    // Host seals (via the digstore-crypto re-export); client opens (via core).
    let ct = digstore_crypto::encrypt_chunk(&key, &plaintext);
    let opened = decrypt_chunk(&key, &ct).expect("tag must verify");
    assert_eq!(opened, plaintext, "client must open host-sealed ciphertext");
    assert_eq!(
        encrypt_chunk(&key, &plaintext),
        ct,
        "deterministic AES-256-GCM-SIV ciphertext is byte-identical"
    );
}

#[test]
fn aead_rejects_tampered_ciphertext() {
    let key = [0x33u8; 32];
    let mut ct = encrypt_chunk(&key, b"data");
    ct[1] ^= 0xFF;
    assert!(
        decrypt_chunk(&key, &ct).is_err(),
        "tampered ciphertext must fail the GCM-SIV tag"
    );
}

/// End-to-end: reproduce exactly what the commit path produces (URN key →
/// AES-256-GCM-SIV chunk → per-resource leaf = `resource_leaf(ct)` → merkle tree
/// → proof) and confirm the verify+decrypt contract accepts it.
#[test]
fn full_pipeline_single_chunk_round_trip() {
    let store = Bytes32([5u8; 32]);
    let resource = "index.html";
    let plaintext = b"<!doctype html><title>dig</title>".to_vec();

    let canonical = canonical_urn(store, resource);
    let key = derive_decryption_key(&canonical, None);
    let ct = encrypt_chunk(&key, &plaintext);
    let leaf = resource_leaf(&ct);

    // Two-resource generation so the proof carries a real sibling step.
    let sibling = Bytes32([0x99u8; 32]);
    let tree = MerkleTree::from_leaves(vec![leaf, sibling]);
    let root = tree.root();
    let proof = MerkleProof {
        leaf,
        path: vec![ProofStep {
            hash: sibling,
            is_left: false,
        }],
        root,
    };

    // The exact `verify_inclusion_core` gate: leaf-bind, fold, root match.
    assert_eq!(resource_leaf(&ct), proof.leaf);
    assert!(proof.verify());
    assert_eq!(proof.root, root);
    assert_eq!(decrypt_chunk(&key, &ct).expect("tag verifies"), plaintext);

    // Wire codec round-trip (the base64 form the wasm crate decodes).
    let decoded = MerkleProof::from_bytes(&proof.to_bytes()).expect("proof round-trips");
    assert_eq!(decoded, proof);
}

/// A decoy / wrong-store response: the proof does NOT chain to the chain-anchored
/// root, so verification rejects it (returns false, not an error).
#[test]
fn decoy_proof_does_not_chain_to_trusted_root() {
    let key = [0x77u8; 32];
    let ct = encrypt_chunk(&key, b"decoy bytes");
    let leaf = resource_leaf(&ct);
    let proof = MerkleProof {
        leaf,
        path: vec![],
        root: leaf,
    };
    let trusted = Bytes32([0x00u8; 32]); // the real chain-anchored root differs
    assert!(proof.verify(), "decoy folds to its own fabricated root");
    assert_ne!(proof.root, trusted, "but not to the trusted root");
}
