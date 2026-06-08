use digstore_core::codec::{Decode, Encode};
use digstore_core::wire::{ChiaBlockRef, ExecutionProof, ProofResponse};
use digstore_core::{Bytes32, Bytes48, Bytes96};

fn sample_proof() -> ExecutionProof {
    ExecutionProof {
        program_hash: Bytes32([1; 32]),
        public_input: vec![1, 2, 3],
        public_output: Bytes32([2; 32]),
        proof: vec![9, 9, 9, 9],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([3; 32]),
            height: 42,
            timestamp: 1_700_000_000,
        },
        node_pubkey: Bytes48([4; 48]),
        node_signature: Bytes96([5; 96]),
    }
}

#[test]
fn chia_block_ref_roundtrip() {
    let b = ChiaBlockRef {
        header_hash: Bytes32([7; 32]),
        height: 1000,
        timestamp: 1_650_000_000,
    };
    assert_eq!(ChiaBlockRef::from_bytes(&b.to_bytes()).unwrap(), b);
}

#[test]
fn execution_proof_roundtrip() {
    let p = sample_proof();
    assert_eq!(ExecutionProof::from_bytes(&p.to_bytes()).unwrap(), p);
}

#[test]
fn proof_response_roundtrip() {
    let r = ProofResponse {
        proof: sample_proof(),
        roothash: Bytes32([8; 32]),
    };
    assert_eq!(ProofResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}
