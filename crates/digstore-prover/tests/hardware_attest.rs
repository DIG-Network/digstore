#![cfg(feature = "hardware-attest")]
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_crypto::bls;
use digstore_prover::build_public_input;
use digstore_prover::hardware::{HardwareAttestProver, HardwareVerifier};
use digstore_prover::{MockChainSource, Prover, ProverError, ServingInputs, Verifier};

#[allow(clippy::type_complexity)]
fn fixture() -> (
    bls::SecretKey,
    bls::PublicKey,
    bls::SecretKey,
    bls::PublicKey,
    ChiaBlockRef,
    Bytes32,
    Bytes32,
    Vec<u8>,
    ServingInputs,
) {
    let node_sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let node_pk = node_sk.public_key();
    let enclave_sk = bls::SecretKey::from_seed(&[42u8; 32]);
    let enclave_pk = enclave_sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving =
        ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1, 2, 3]] };
    (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving)
}

#[test]
fn hardware_attest_round_trip() {
    let (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving) = fixture();
    let proof =
        HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    HardwareVerifier::new(enclave_pk.to_bytes()).verify(&proof, ph, &[root], &chain).unwrap();
}

#[test]
fn tampered_attestation_rejected() {
    let (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving) = fixture();
    let mut proof =
        HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    proof.proof[0] ^= 0xFF;
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = HardwareVerifier::new(enclave_pk.to_bytes()).verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::AttestationInvalid(_)));
}

#[test]
fn wrong_enclave_key_rejected() {
    let (node_sk, node_pk, enclave_sk, _enclave_pk, block, ph, root, pi, serving) = fixture();
    let proof =
        HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let wrong = bls::SecretKey::from_seed(&[7u8; 32]).public_key().to_bytes();
    let err = HardwareVerifier::new(wrong).verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::AttestationInvalid(_)));
}
