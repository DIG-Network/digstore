use digstore_core::merkle::MerkleTree;
use digstore_core::Bytes32;
use digstore_guest::content::emit_merkle_proof;

#[test]
fn emitted_proof_verifies_against_core() {
    // Four chunks -> leaves = SHA-256(chunk). Build the core tree, then emit a
    // proof for chunk index 2 inside the guest and verify it with core rules.
    let chunks: Vec<Vec<u8>> = vec![
        b"alpha".to_vec(),
        b"beta".to_vec(),
        b"gamma".to_vec(),
        b"delta".to_vec(),
    ];
    let tree = MerkleTree::build(&chunks);
    let root: Bytes32 = tree.root();

    let proof = emit_merkle_proof(&tree, 2);
    assert_eq!(proof.root, root);
    assert!(proof.verify(), "guest-emitted proof must verify under core rules");
}

mod fixtures;
mod mock_host;
use digstore_core::{ContentResponse, KeyTableEntry};
use digstore_guest::content::{serve_content, ContentOutcome, GateConfig};
use digstore_guest::datasection::{encode_key_table, DataSection};
use digstore_guest::request::{ContentRequest, ValidityWindow};
use mock_host::MockHost;

fn gate_config() -> GateConfig {
    GateConfig {
        require_attestation: false,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    }
}

#[test]
fn hit_returns_real_content_response() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0, 1, 2, 3],
        total_size: 20,
    };
    let table = encode_key_table(&[entry]);
    // Pool stores 4 chunk ciphertexts of fixed 5 bytes each in section ChunkPool.
    let pool = fixtures::pack_pool(&[b"alpha", b"beta_", b"gamma", b"delta"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();

    let host = MockHost::default();
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    match serve_content(&host, &ds, &req, &gate_config()) {
        ContentOutcome::Real(resp) => {
            let r: ContentResponse = resp;
            assert!(!r.ciphertext.is_empty());
            assert_eq!(r.roothash, ds.current_root());
        }
        ContentOutcome::Decoy(_) => panic!("hit must return Real, not Decoy"),
    }
}

#[test]
fn miss_returns_decoy() {
    let table = encode_key_table(&[]); // empty table => every key misses
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &[]);
    let ds = DataSection::parse(&blob).unwrap();
    let host = MockHost::default();
    let req = ContentRequest {
        retrieval_key: Bytes32([0x99; 32]),
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gate_config()),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn outside_temporal_window_returns_decoy_even_on_hit() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let mut host = MockHost::default();
    host.time = 50; // before window
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: Some(ValidityWindow { not_before: 100, not_after: 200 }),
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gate_config()),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn failed_attestation_returns_decoy() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let mut host = MockHost::default();
    host.attestation = Err(digstore_core::ErrorCode::AttestationFailed);
    let mut gc = gate_config();
    gc.require_attestation = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gc),
        ContentOutcome::Decoy(_)
    ));
}
