use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_crypto::bls;
use digstore_prover::{
    build_public_input, MockChainSource, MockProver, MockVerifier, Prover, ServingInputs, Verifier,
};

fn block() -> ChiaBlockRef {
    ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 42,
        timestamp: 1_000_000,
    }
}

#[test]
fn mock_prove_verify_round_trip() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = block();

    let program_hash = Bytes32([0xAAu8; 32]);
    let roothash = Bytes32([0xBBu8; 32]);
    let nonce = [0x33u8; 32];
    let public_input = build_public_input(&nonce, &block);

    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash,
        chunk_ciphertext: vec![vec![0xDEu8, 0xAD], vec![0xBE, 0xEF]],
    };

    let prover = MockProver::new(sk, pk.clone(), block.clone());
    let proof = prover.prove(program_hash, &public_input, &serving).unwrap();

    assert_eq!(proof.node_pubkey, pk.to_bytes());
    assert_eq!(proof.chia_block, block);
    assert_eq!(proof.program_hash, program_hash);
    assert_eq!(proof.public_output, serving.compute_public_output());

    let chain = MockChainSource::new(vec![block.clone()], 1_000_030);
    MockVerifier
        .verify(&proof, program_hash, &[roothash], &chain)
        .expect("genuine mock proof must verify");
}
