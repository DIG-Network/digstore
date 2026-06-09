use digstore_core::{Bytes32, Bytes48, ChiaBlockRef, ExecutionProof};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

fn fresh_proof() -> (ExecutionProof, Bytes32, Bytes32, ChiaBlockRef) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 42,
        timestamp: 1_000_000,
    };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash: root,
        chunk_ciphertext: vec![vec![5, 5]],
    };
    let proof = MockProver::new(sk, pk, block.clone())
        .prove(ph, &public_input, &serving)
        .unwrap();
    (proof, ph, root, block)
}

#[test]
fn genuine_signature_attributes_to_node() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .unwrap();
}

#[test]
fn tampered_signature_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    proof.node_signature.0[0] ^= 0xFF;
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}

#[test]
fn wrong_pubkey_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    let other_pk = bls::SecretKey::from_seed(&[99u8; 32]).public_key();
    proof.node_pubkey = other_pk.to_bytes();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}

// §13.7 + §12.2: the serving node is identified by the BLS key it already uses
// for attestation, "one key for both roles". A proof whose node_pubkey is NOT a
// member of the module's embedded §12 attestation trusted-key set MUST be
// rejected, so the "one key for both roles" guarantee is structural, not a
// matter of convention.

#[test]
fn node_pubkey_in_attestation_trusted_set_is_accepted() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    // The node's own attestation key (the [7u8;32] seed used by fresh_proof) is
    // the trusted attestation key embedded in the module.
    let node_pk = bls::SecretKey::from_seed(&[7u8; 32]).public_key();
    let trusted_node_keys = [node_pk.to_bytes()];
    MockVerifier::default()
        .verify_node_attested(&proof, ph, &[root], &trusted_node_keys, &chain)
        .expect("a proof signed by an attestation-trusted node key must verify");
}

#[test]
fn node_pubkey_not_in_attestation_trusted_set_is_rejected() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    // A DIFFERENT key is the only trusted attestation key. The proof carries a
    // genuine signature under its own node key, so signature verification passes,
    // but the node key is not bound to the §12 attestation set: reject.
    let other_pk = bls::SecretKey::from_seed(&[0xABu8; 32]).public_key();
    let trusted_node_keys: [Bytes48; 1] = [other_pk.to_bytes()];
    let err = MockVerifier::default()
        .verify_node_attested(&proof, ph, &[root], &trusted_node_keys, &chain)
        .unwrap_err();
    assert!(
        matches!(err, ProverError::NodeKeyNotAttested(_)),
        "expected NodeKeyNotAttested, got {err:?}"
    );
}

#[test]
fn empty_attestation_trusted_set_is_rejected() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default()
        .verify_node_attested(&proof, ph, &[root], &[], &chain)
        .unwrap_err();
    assert!(
        matches!(err, ProverError::NodeKeyNotAttested(_)),
        "expected NodeKeyNotAttested for empty trusted set, got {err:?}"
    );
}
