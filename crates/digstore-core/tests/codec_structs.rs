use digstore_core::codec::section::{SectionEntry, SectionHeader, FORMAT_VERSION};
use digstore_core::codec::{Decode, Encode};
use digstore_core::keytable::{KeyTableEntry, PathWalk};
use digstore_core::merkle::{MerkleProof, ProofStep};
use digstore_core::urn::Urn;
use digstore_core::wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ChiaBlockRef, ContentResponse,
    ExecutionProof, ProofResponse,
};
use digstore_core::{Bytes32, Bytes48, Bytes96};

fn assert_roundtrip<T: Encode + Decode + PartialEq + core::fmt::Debug>(value: T) {
    let bytes = value.to_bytes();
    let decoded = T::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded, value);
}

#[test]
fn all_structs_roundtrip() {
    assert_roundtrip(Urn {
        chain: "mainnet".into(),
        store_id: Bytes32([1; 32]),
        root_hash: Some(Bytes32([2; 32])),
        resource_key: Some("a/b".into()),
    });
    assert_roundtrip(SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![SectionEntry { id: 1, offset: 0, length: 8 }],
    });
    assert_roundtrip(KeyTableEntry {
        static_key: Bytes32([3; 32]),
        generation: Bytes32([4; 32]),
        chunk_indices: vec![1, 2, 3],
        total_size: 99,
    });
    assert_roundtrip(PathWalk {
        resource_key: Bytes32([5; 32]),
        chunk_indices: vec![7],
        cursor: 0,
    });
    let mp = MerkleProof {
        leaf: Bytes32([6; 32]),
        path: vec![ProofStep { hash: Bytes32([7; 32]), is_left: false }],
        root: Bytes32([8; 32]),
    };
    assert_roundtrip(mp.clone());
    assert_roundtrip(ChiaBlockRef {
        header_hash: Bytes32([9; 32]),
        height: 1,
        timestamp: 2,
    });
    let ep = ExecutionProof {
        program_hash: Bytes32([10; 32]),
        public_input: vec![1, 2],
        public_output: Bytes32([11; 32]),
        proof: vec![3, 4],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([12; 32]),
            height: 5,
            timestamp: 6,
        },
        node_pubkey: Bytes48([13; 48]),
        node_signature: Bytes96([14; 96]),
    };
    assert_roundtrip(ep.clone());
    assert_roundtrip(ProofResponse { proof: ep, roothash: Bytes32([15; 32]) });
    assert_roundtrip(ContentResponse {
        ciphertext: vec![1, 2, 3],
        merkle_proof: mp,
        roothash: Bytes32([16; 32]),
    });
    assert_roundtrip(AttestationChallenge { nonce: [1; 32], store_id: [2; 32], timestamp: 3 });
    assert_roundtrip(AttestationResponse {
        host_public_key: [4; 48],
        host_instance_id: [5; 32],
        signature: [6; 96],
    });
    assert_roundtrip(AuthenticationInfo {
        requires_session: true,
        requires_jwt: true,
        jwks_url: None,
        accepted_algorithms: vec!["RS256".into()],
    });
}

#[test]
fn content_response_empty_golden() {
    let r = ContentResponse {
        ciphertext: vec![],
        merkle_proof: MerkleProof {
            leaf: Bytes32([0; 32]),
            path: vec![],
            root: Bytes32([0; 32]),
        },
        roothash: Bytes32([0; 32]),
    };
    let bytes = r.to_bytes();
    // ciphertext: 4-byte count(0) = 0,0,0,0
    // merkle_proof.leaf: 32 zero bytes
    // merkle_proof.path: 4-byte count(0) = 0,0,0,0
    // merkle_proof.root: 32 zero bytes
    // roothash: 32 zero bytes
    assert_eq!(bytes.len(), 4 + 32 + 4 + 32 + 32);
    assert_eq!(&bytes[0..4], &[0, 0, 0, 0]);
}

#[test]
fn execution_proof_field_order_golden() {
    let ep = ExecutionProof {
        program_hash: Bytes32([0xAA; 32]),
        public_input: vec![],
        public_output: Bytes32([0xBB; 32]),
        proof: vec![],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([0xCC; 32]),
            height: 0,
            timestamp: 0,
        },
        node_pubkey: Bytes48([0xDD; 48]),
        node_signature: Bytes96([0xEE; 96]),
    };
    let bytes = ep.to_bytes();
    // program_hash first.
    assert_eq!(&bytes[0..32], &[0xAA; 32]);
    // then public_input length (0) BE.
    assert_eq!(&bytes[32..36], &[0, 0, 0, 0]);
    // then public_output (0xBB * 32).
    assert_eq!(&bytes[36..68], &[0xBB; 32]);
}
