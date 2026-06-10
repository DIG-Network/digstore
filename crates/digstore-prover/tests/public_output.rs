use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

#[test]
fn tampered_public_output_is_rejected() {
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
        chunk_ciphertext: vec![vec![1, 2, 3]],
    };
    let mut proof = MockProver::new(sk, pk, block.clone())
        .prove(ph, &public_input, &serving)
        .unwrap();
    proof.public_output = Bytes32([0xEEu8; 32]); // tamper the committed output
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier
        .verify(&proof, ph, &[root], &chain)
        .unwrap_err();
    // In the mock, public_output feeds the commitment chain, so tampering it breaks
    // the recompute and surfaces as ZkProofInvalid. (Risc0Verifier surfaces the same
    // tamper as PublicOutputMismatch via the journal — both are valid §13.4 rejections.)
    assert!(matches!(err, ProverError::ZkProofInvalid(_)));
}

#[test]
fn output_bytes_differ_changes_commitment() {
    let a = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash: Bytes32([2u8; 32]),
        chunk_ciphertext: vec![vec![1, 2, 3]],
    };
    let b = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash: Bytes32([2u8; 32]),
        chunk_ciphertext: vec![vec![1, 2, 4]],
    };
    assert_ne!(a.compute_public_output(), b.compute_public_output());
}
