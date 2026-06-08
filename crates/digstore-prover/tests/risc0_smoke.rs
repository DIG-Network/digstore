#![cfg(feature = "risc0")]
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_crypto::bls;
use digstore_prover::build_public_input;
use digstore_prover::risc0_backend::{Risc0Prover, Risc0Verifier};
use digstore_prover::{MockChainSource, Prover, ProverError, ServingInputs, Verifier};

/// Real risc0 prove->verify. Slow; opt in with `--ignored`. In dev mode
/// (`RISC0_DEV_MODE=1`) it runs in seconds. This is a REAL test, not a stub.
#[test]
#[ignore = "slow: real risc0 proving; run with --ignored or RISC0_DEV_MODE=1"]
fn risc0_prove_verify_smoke() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash: root,
        chunk_ciphertext: vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]],
    };

    let proof = Risc0Prover::new(sk, pk, block.clone()).prove(ph, &pi, &serving).expect("risc0 proving must succeed");
    assert_eq!(proof.public_output, serving.compute_public_output());

    let chain = MockChainSource::new(vec![block], 1_000_100);
    Risc0Verifier::default().verify(&proof, ph, &[root], &chain).expect("risc0 proof must verify");
}

#[test]
#[ignore = "slow: real risc0 proving"]
fn risc0_tampered_output_rejected() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving =
        ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1, 2, 3]] };
    let mut proof = Risc0Prover::new(sk, pk, block.clone()).prove(ph, &pi, &serving).unwrap();
    proof.public_output = Bytes32([0xEEu8; 32]); // tamper claimed output
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = Risc0Verifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::PublicOutputMismatch));
}
