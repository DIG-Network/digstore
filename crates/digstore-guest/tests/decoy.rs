use digstore_core::Bytes32;
use digstore_guest::decoy::{decoy_bytes, decoy_size};

#[test]
fn same_key_same_bytes() {
    let k = Bytes32([0x42; 32]);
    let a = decoy_bytes(&k);
    let b = decoy_bytes(&k);
    assert_eq!(a, b, "decoy bytes must be deterministic for a fixed retrieval key");
}

#[test]
fn different_key_different_bytes() {
    let a = decoy_bytes(&Bytes32([1; 32]));
    let b = decoy_bytes(&Bytes32([2; 32]));
    assert_ne!(a, b);
}

#[test]
fn size_is_deterministic_and_in_log_band() {
    // Logarithmic distribution: sizes cluster in [1KiB, 256KiB].
    let k = Bytes32([0x7E; 32]);
    let s1 = decoy_size(&k);
    let s2 = decoy_size(&k);
    assert_eq!(s1, s2, "size must be deterministic per key");
    assert!((1024..=256 * 1024).contains(&s1), "size {s1} out of log band");
    assert_eq!(decoy_bytes(&k).len(), s1, "byte length must equal decoy_size");
}

#[test]
fn distribution_spreads_across_buckets() {
    // Across many keys, sizes must not all collapse to one value.
    let mut sizes = std::collections::BTreeSet::new();
    for i in 0..200u8 {
        sizes.insert(decoy_size(&Bytes32([i; 32])));
    }
    assert!(sizes.len() > 10, "expected varied decoy sizes, got {}", sizes.len());
}

use digstore_core::{ContentResponse, MerkleProof, ProofStep};
use digstore_guest::decoy::decoy_content_response;

#[test]
fn decoy_content_response_has_real_field_shape() {
    let k = Bytes32([0xC0; 32]);
    let root = Bytes32([0xD0; 32]);
    let resp: ContentResponse = decoy_content_response(&k, &root);
    // Same struct as a real hit: ciphertext + merkle_proof + roothash.
    assert_eq!(resp.roothash, root, "decoy must carry the requested/current root");
    assert_eq!(resp.ciphertext, decoy_content_response(&k, &root).ciphertext, "deterministic");
    // Proof blob is a well-formed MerkleProof value (leaf, non-empty path, root).
    let p: &MerkleProof = &resp.merkle_proof;
    assert_eq!(p.root, root);
    assert!(!p.path.is_empty(), "decoy proof must have a path like a real one");
    // Each ProofStep is well formed.
    let _: &ProofStep = &p.path[0];
    assert!(!resp.ciphertext.is_empty());
}
