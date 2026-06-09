//! D5: the guest's served merkle proof must genuinely VERIFY.
//!
//! Construct a data-section blob with 2 resources (KeyTable + ChunkPool +
//! MerkleNodes + CurrentRoot), serve one, and assert the emitted
//! `ContentResponse.merkle_proof`:
//!   * `proof.verify() == true` (recomputes the injected current root), and
//!   * `proof.root == injected current_root`, and
//!   * `proof.leaf == SHA-256(served ciphertext)` (per-resource leaf, D5).

mod fixtures;
mod mock_host;

use digstore_core::datasection::{encode_blob, encode_merkle_nodes, SectionId};
use digstore_core::merkle::MerkleTree;
use digstore_core::serving::concat_output;
use digstore_core::{Bytes32, KeyTableEntry};
use digstore_guest::content::{serve_content, ContentOutcome, GateConfig};
use digstore_guest::datasection::{encode_key_table, DataSection};
use digstore_guest::request::ContentRequest;
use mock_host::MockHost;
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(bytes);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h.finalize());
    Bytes32(o)
}

fn gate_config() -> GateConfig {
    GateConfig {
        require_attestation: false,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    }
}

/// Pack a chunk pool body: count(u32 BE) then per chunk: len(u32 BE) || bytes.
fn pack_pool(chunks: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(chunks.len() as u32).to_be_bytes());
    for c in chunks {
        out.extend_from_slice(&(c.len() as u32).to_be_bytes());
        out.extend_from_slice(c);
    }
    out
}

#[test]
fn served_proof_verifies_against_injected_current_root() {
    // Two resources. Resource A (static_key 0x11..) uses pool chunks [0,1];
    // resource B (static_key 0x22..) uses pool chunk [2].
    let key_a = Bytes32([0x11; 32]);
    let key_b = Bytes32([0x22; 32]);

    // Global chunk pool (global-index order): 3 distinct ciphertext chunks.
    let c0: &[u8] = b"alpha-ciphertext";
    let c1: &[u8] = b"beta-ciphertext";
    let c2: &[u8] = b"gamma-ciphertext";
    let pool = pack_pool(&[c0, c1, c2]);

    let entry_a = KeyTableEntry {
        static_key: key_a,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0, 1],
        total_size: (c0.len() + c1.len()) as u64,
    };
    let entry_b = KeyTableEntry {
        static_key: key_b,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![2],
        total_size: c2.len() as u64,
    };

    // Leaves: one per resource, ascending by static_key.
    // key_a (0x11) < key_b (0x22), so leaf order is [A, B].
    let blob_a = concat_output(&[c0, c1]); // exactly what get_content returns for A
    let blob_b = concat_output(&[c2]);
    let leaf_a = sha256(&blob_a);
    let leaf_b = sha256(&blob_b);
    let leaves = vec![leaf_a, leaf_b];

    let tree = MerkleTree::from_leaves(leaves.clone());
    let current_root = tree.root();

    // KeyTable order = leaf order (ascending static_key): [A, B].
    let key_table = encode_key_table(&[entry_a.clone(), entry_b.clone()]);
    let merkle_nodes = encode_merkle_nodes(&leaves);

    let sections: Vec<(u16, Vec<u8>)> = vec![
        (SectionId::StoreId as u16, [0xAA; 32].to_vec()),
        (SectionId::CurrentRoot as u16, current_root.0.to_vec()),
        (SectionId::KeyTable as u16, key_table),
        (SectionId::ChunkPool as u16, pool),
        (SectionId::MerkleNodes as u16, merkle_nodes),
    ];
    let blob = encode_blob(&sections);
    let ds = DataSection::parse(&blob).unwrap();
    assert_eq!(
        ds.current_root(),
        current_root,
        "injected root must round-trip"
    );

    let host = MockHost::default();

    // Serve resource A.
    let req_a = ContentRequest {
        retrieval_key: key_a,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    match serve_content(&host, &ds, &req_a, &gate_config()) {
        ContentOutcome::Real(resp) => {
            assert_eq!(resp.roothash, current_root);
            assert_eq!(
                resp.merkle_proof.root, current_root,
                "proof.root must equal the injected current_root"
            );
            assert_eq!(
                resp.merkle_proof.leaf,
                sha256(&resp.ciphertext),
                "leaf must be SHA-256 of the served ciphertext (per-resource leaf, D5)"
            );
            assert!(
                resp.merkle_proof.verify(),
                "emitted proof must verify against the injected current root"
            );
        }
        ContentOutcome::Decoy(_) => panic!("hit on resource A must return Real, not Decoy"),
    }

    // Serve resource B (index 1) and verify too.
    let req_b = ContentRequest {
        retrieval_key: key_b,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    match serve_content(&host, &ds, &req_b, &gate_config()) {
        ContentOutcome::Real(resp) => {
            assert_eq!(resp.merkle_proof.root, current_root);
            assert_eq!(resp.merkle_proof.leaf, sha256(&resp.ciphertext));
            assert!(
                resp.merkle_proof.verify(),
                "proof for resource B must also verify"
            );
        }
        ContentOutcome::Decoy(_) => panic!("hit on resource B must return Real, not Decoy"),
    }
}
