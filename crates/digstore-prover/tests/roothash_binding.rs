use digstore_core::{Bytes32, ChiaBlockRef, ProofResponse};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

fn make_response(roothash: Bytes32) -> (ProofResponse, Bytes32, ChiaBlockRef, Vec<u8>) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let program_hash = Bytes32([0xAAu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash,
        chunk_ciphertext: vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]],
    };
    let returned = serving.output_bytes();
    let proof = MockProver::new(sk, pk, block.clone())
        .prove(program_hash, &public_input, &serving)
        .unwrap();
    (ProofResponse { proof, roothash }, program_hash, block, returned)
}

#[test]
fn bound_roothash_is_accepted() {
    let root = Bytes32([0xBBu8; 32]);
    let (resp, ph, block, returned) = make_response(root);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    MockVerifier::default()
        .verify_response(&resp, ph, &[root], &returned, &chain)
        .expect("a proof bound to the asserted trusted root must verify");
}

#[test]
fn untrusted_roothash_is_rejected() {
    let root = Bytes32([0xBBu8; 32]);
    let (resp, ph, block, returned) = make_response(root);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let other = Bytes32([0xCCu8; 32]);
    let err = MockVerifier::default()
        .verify_response(&resp, ph, &[other], &returned, &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::UntrustedRoot(_)));
}

#[test]
fn proof_bound_to_root_b_rejected_when_response_asserts_different_trusted_root_c() {
    // Genuine proof bound to root B; attacker swaps the response root to C,
    // and the verifier happens to trust BOTH B and C. The binding check must
    // still reject because public_output committed B, not C.
    let root_b = Bytes32([0xBBu8; 32]);
    let root_c = Bytes32([0xCCu8; 32]);
    let (mut resp, ph, block, returned) = make_response(root_b);
    resp.roothash = root_c; // forge the asserted root
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let err = MockVerifier::default()
        .verify_response(&resp, ph, &[root_b, root_c], &returned, &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::RootBindingMismatch { .. }));
}
