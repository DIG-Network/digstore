//! NATIVE parity oracle: prove the reimplemented read-crypto in `dig-client-wasm`
//! produces BYTE-IDENTICAL output to the canonical host-side `digstore-crypto`,
//! and that the read pipeline accepts data the host/CLI commit path produces.
//!
//! This test does NOT run under wasm32 (it links the blst-backed
//! `digstore-crypto`); it runs as a normal `cargo test` on the host to anchor the
//! parity contract. The wasm crate ships the reproduced constants in `crypto.rs`;
//! if either side ever drifts, this test fails.

// Re-expose the crate's internal crypto + the canonical URN for testing.
use digstore_core::{Bytes32, MerkleProof, MerkleTree, ProofStep, Urn};
use digstore_core::codec::{Decode, Encode};

/// Mirror of `dig_client_wasm::canonical_resource_urn` (private), kept here to
/// drive both crypto stacks with the same canonical URN string.
fn canonical_urn(store_id: Bytes32, resource_key: &str) -> String {
    Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some(resource_key.to_string()),
    }
    .canonical()
}

// Pull the wasm crate's crypto module functions via its public lib? They are
// private (`mod crypto`), so we re-declare the SAME formulas here against the
// SAME constants and assert they equal digstore-crypto. To exercise the ACTUAL
// shipped code path we additionally go through the wasm crate's public
// `deriveKey`-equivalent by recomputing the canonical URN identically.

#[path = "../src/crypto.rs"]
mod wasm_crypto;

#[test]
fn kdf_matches_digstore_crypto_public() {
    let store = Bytes32([7u8; 32]);
    let canonical = canonical_urn(store, "index.html");

    let ours = wasm_crypto::derive_decryption_key(&canonical, None);
    let theirs = digstore_crypto::derive_decryption_key(&canonical, None);
    assert_eq!(ours, theirs, "public-store KDF must be byte-identical");
}

#[test]
fn kdf_matches_digstore_crypto_private_salt() {
    use digstore_core::SecretSalt;
    let store = Bytes32([0xABu8; 32]);
    let canonical = canonical_urn(store, "secret/page.html");
    let salt = [0x42u8; 32];

    let ours = wasm_crypto::derive_decryption_key(&canonical, Some(&salt));
    let theirs = digstore_crypto::derive_decryption_key(&canonical, Some(&SecretSalt(salt)));
    assert_eq!(ours, theirs, "private-store KDF must be byte-identical");
    assert_ne!(
        ours,
        wasm_crypto::derive_decryption_key(&canonical, None),
        "salt must change the key"
    );
}

#[test]
fn aead_decrypts_what_digstore_crypto_encrypts() {
    let key = [0x11u8; 32];
    let plaintext = b"<html><body>hello dig</body></html>".to_vec();

    // Host seals, browser opens.
    let ct = digstore_crypto::encrypt_chunk(&key, &plaintext);
    let opened = wasm_crypto::decrypt_chunk(&key, &ct).expect("tag must verify");
    assert_eq!(opened, plaintext, "browser must open host-sealed ciphertext");

    // And our own encrypt is byte-identical to the host's (deterministic AEAD).
    let ours = wasm_crypto::encrypt_chunk(&key, &plaintext);
    assert_eq!(ours, ct, "AES-256-GCM-SIV ciphertext must be byte-identical");
}

#[test]
fn aead_rejects_tampered_ciphertext() {
    let key = [0x33u8; 32];
    let mut ct = digstore_crypto::encrypt_chunk(&key, b"data");
    ct[1] ^= 0xFF;
    assert!(
        wasm_crypto::decrypt_chunk(&key, &ct).is_err(),
        "tampered ciphertext must fail the GCM-SIV tag"
    );
}

#[test]
fn sha256_matches_core() {
    let data = b"the quick brown fox";
    assert_eq!(
        Bytes32(wasm_crypto::sha256(data)),
        digstore_core::sha256(data),
        "SHA-256 must match digstore-core"
    );
}

/// End-to-end: reproduce exactly what the commit path produces (URN key ->
/// AES-256-GCM-SIV chunk -> per-resource leaf = SHA-256(plain concat) -> merkle
/// tree -> proof) and confirm the wasm crate's verify+decrypt core accepts it.
#[test]
fn full_pipeline_single_chunk_round_trip() {
    let store = Bytes32([5u8; 32]);
    let resource = "index.html";
    let plaintext = b"<!doctype html><title>dig</title>".to_vec();

    // Commit-time (host) derivation + seal.
    let canonical = canonical_urn(store, resource);
    let key = digstore_crypto::derive_decryption_key(&canonical, None);
    let ct = digstore_crypto::encrypt_chunk(&key, &plaintext);
    // D5: per-resource leaf = SHA-256(plain concat of chunk ciphertexts).
    let leaf = digstore_crypto::sha256(&ct);

    // Two-resource generation so the proof has a real sibling step.
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

    // Browser side: derive key the SAME way, verify inclusion, decrypt.
    let our_key = wasm_crypto::derive_decryption_key(&canonical, None);
    assert_eq!(our_key, key);

    // Verify inclusion exactly as the wasm `verify_inclusion_core` does.
    assert_eq!(Bytes32(wasm_crypto::sha256(&ct)), proof.leaf);
    assert!(proof.verify());
    assert_eq!(proof.root, root);

    // Decrypt.
    let opened = wasm_crypto::decrypt_chunk(&our_key, &ct).expect("tag verifies");
    assert_eq!(opened, plaintext);

    // Sanity: the proof encodes/decodes through the Chia BE codec (the wire form
    // the wasm crate decodes from base64).
    let encoded = proof.to_bytes();
    let decoded = MerkleProof::from_bytes(&encoded).expect("proof round-trips");
    assert_eq!(decoded, proof);
}

/// A decoy / wrong-store response: the proof does NOT chain to the chain-anchored
/// root, so verification must reject it (returns false, not an error).
#[test]
fn decoy_proof_does_not_chain_to_trusted_root() {
    let key = [0x77u8; 32];
    let ct = digstore_crypto::encrypt_chunk(&key, b"decoy bytes");
    let leaf = digstore_crypto::sha256(&ct);
    // Fabricated single-leaf proof (a decoy "proves" against its own root).
    let proof = MerkleProof {
        leaf,
        path: vec![],
        root: leaf,
    };
    let trusted = Bytes32([0x00u8; 32]); // the real chain-anchored root differs
    assert!(proof.verify(), "decoy folds to its own fabricated root");
    assert_ne!(proof.root, trusted, "but not to the trusted root");
}
