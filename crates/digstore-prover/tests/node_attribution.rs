use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

fn fresh_proof() -> (ExecutionProof, Bytes32, Bytes32, ChiaBlockRef) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving =
        ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![5, 5]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(ph, &public_input, &serving).unwrap();
    (proof, ph, root, block)
}

#[test]
fn genuine_signature_attributes_to_node() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap();
}

#[test]
fn tampered_signature_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    proof.node_signature.0[0] ^= 0xFF;
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}

#[test]
fn wrong_pubkey_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    let other_pk = bls::SecretKey::from_seed(&[99u8; 32]).public_key();
    proof.node_pubkey = other_pk.to_bytes();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}
