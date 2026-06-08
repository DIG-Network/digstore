use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, ChainSource, MockChainSource, MockProver, MockVerifier, Prover,
    ServingInputs, Verifier,
};

#[test]
fn prover_and_verifier_are_object_safe() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let prover: Box<dyn Prover> = Box::new(MockProver::new(sk, pk, block.clone()));
    let verifier: Box<dyn Verifier> = Box::new(MockVerifier::default());
    let chain: Box<dyn ChainSource> = Box::new(MockChainSource::new(vec![block.clone()], 1_000_100));

    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving =
        ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1]] };
    let proof = prover.prove(ph, &pi, &serving).unwrap();
    verifier.verify(&proof, ph, &[root], chain.as_ref()).unwrap();
}
