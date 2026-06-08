use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

fn make_proof(nonce: [u8; 32]) -> (ExecutionProof, Bytes32, Bytes32, ChiaBlockRef) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let program_hash = Bytes32([0xAAu8; 32]);
    let roothash = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&nonce, &block);
    let serving =
        ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash, chunk_ciphertext: vec![vec![9, 9, 9]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(program_hash, &public_input, &serving).unwrap();
    (proof, program_hash, roothash, block)
}

#[test]
fn proof_for_nonce_a_rejected_against_request_nonce_b() {
    let (proof, ph, root, block) = make_proof([0xA1u8; 32]);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let err = MockVerifier::default()
        .verify_with_nonce(&proof, &[0xB2u8; 32], ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::NonceMismatch));
}

#[test]
fn proof_for_nonce_a_accepted_against_request_nonce_a() {
    let nonce_a = [0xA1u8; 32];
    let (proof, ph, root, block) = make_proof(nonce_a);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    MockVerifier::default()
        .verify_with_nonce(&proof, &nonce_a, ph, &[root], &chain)
        .expect("matching nonce must verify");
}
