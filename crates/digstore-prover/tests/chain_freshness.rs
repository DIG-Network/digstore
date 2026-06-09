use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ProverError,
    ServingInputs, Verifier,
};

fn proof_bound_to(block: &ChiaBlockRef) -> (ExecutionProof, Bytes32, Bytes32) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], block);
    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash: root,
        chunk_ciphertext: vec![vec![1]],
    };
    let proof = MockProver::new(sk, pk, block.clone())
        .prove(ph, &public_input, &serving)
        .unwrap();
    (proof, ph, root)
}

#[test]
fn block_inside_freshness_window_accepted() {
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 42,
        timestamp: 1_000_000,
    };
    let (proof, ph, root) = proof_bound_to(&block);
    let chain = MockChainSource::new(vec![block], 1_000_300); // 300s < 600s window
    MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .expect("fresh block accepted");
}

#[test]
fn block_outside_freshness_window_rejected() {
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 42,
        timestamp: 1_000_000,
    };
    let (proof, ph, root) = proof_bound_to(&block);
    let chain = MockChainSource::new(vec![block], 1_000_700); // 700s > 600s window
    let err = MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::BlockTooOld { .. }));
}

#[test]
fn block_unknown_to_chain_rejected() {
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 42,
        timestamp: 1_000_000,
    };
    let (proof, ph, root) = proof_bound_to(&block);
    let other = ChiaBlockRef {
        header_hash: Bytes32([0x66u8; 32]),
        height: 43,
        timestamp: 1_000_010,
    };
    let chain = MockChainSource::new(vec![other], 1_000_300);
    let err = MockVerifier::default()
        .verify(&proof, ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::BlockNotOnChain(_)));
}
