//! Golden + structural test for the contract data-section blob (D1–D5).
//!
//! The byte-exact format is owned by `digstore_core::datasection`. This test
//! pins a known-good blob and independently re-parses every section through the
//! canonical `DataView` so the compiler's emitted bytes are exactly what the
//! guest reads and the client verifies.

mod common;

use digstore_compiler::{encode_data_section, DataSectionInputs};
use digstore_core::datasection::{
    decode_merkle_leaves, lookup_key, read_chunk, DataView, SectionId,
};
use digstore_core::merkle::MerkleTree;
use digstore_core::{
    AuthenticationInfo, Bytes32, Bytes48, Decode, Decoder, KeyTableEntry, MetadataManifest,
    TrustedHostKey,
};

fn fixed_inputs() -> DataSectionInputs {
    // Two resource leaves, ascending by static_key; current_root = tree root.
    let leaves = vec![Bytes32([0x33; 32]), Bytes32([0x44; 32])];
    let current_root = MerkleTree::from_leaves(leaves.clone()).root();
    DataSectionInputs {
        store_id: Bytes32([0xAB; 32]),
        current_root,
        root_history: vec![Bytes32([0x11; 32])],
        store_pubkey: Bytes48([0xCD; 48]),
        trusted_keys: vec![TrustedHostKey {
            public_key: [0x42u8; 48],
            label: "L".into(),
        }],
        manifest: common::sample_manifest(),
        auth_info: AuthenticationInfo {
            requires_session: false,
            requires_jwt: false,
            jwks_url: None,
            accepted_algorithms: vec![],
        },
        key_table: vec![KeyTableEntry {
            static_key: Bytes32([0x01; 32]),
            generation: Bytes32([0x11; 32]),
            chunk_indices: vec![0],
            total_size: 6,
        }],
        chunk_pool_bodies: vec![b"abcdef".to_vec()],
        merkle_leaves: leaves,
        filler: vec![0x09; 16],
    }
}

#[test]
fn structure_is_independently_valid() {
    let inp = fixed_inputs();
    let blob = encode_data_section(&inp);

    // Header (D1): magic + version 1.
    assert_eq!(&blob[0..4], b"DIGS");
    assert_eq!(blob[4], 1u8);

    let view = DataView::parse(&blob).expect("blob parses through canonical DataView");

    // Identity sections are raw bytes.
    assert_eq!(view.section(SectionId::StoreId).unwrap(), &inp.store_id.0);
    assert_eq!(
        view.section(SectionId::CurrentRoot).unwrap(),
        &inp.current_root.0
    );
    assert_eq!(
        view.section(SectionId::PublicKey).unwrap(),
        &inp.store_pubkey.0
    );

    // RootHistory: Vec<Bytes32> framing.
    let rh = view.section(SectionId::RootHistory).unwrap();
    let mut dec = Decoder::new(rh);
    let hist = Vec::<Bytes32>::decode(&mut dec).unwrap();
    assert_eq!(hist, inp.root_history);

    // Metadata: MetadataManifest.
    let md = view.section(SectionId::Metadata).unwrap();
    let mut dec = Decoder::new(md);
    let m = MetadataManifest::decode(&mut dec).unwrap();
    assert_eq!(m.name, "sample-store");

    // AuthInfo round-trips.
    let ai = view.section(SectionId::AuthInfo).unwrap();
    let mut dec = Decoder::new(ai);
    let a = AuthenticationInfo::decode(&mut dec).unwrap();
    assert_eq!(a, inp.auth_info);

    // TrustedKeys: count(u32) then [u8;48] + String label.
    let tk = view.section(SectionId::TrustedKeys).unwrap();
    assert_eq!(u32::from_be_bytes([tk[0], tk[1], tk[2], tk[3]]), 1);

    // KeyTable: lookup by static_key.
    let kt = view.section(SectionId::KeyTable).unwrap();
    let entry = lookup_key(kt, &Bytes32([0x01; 32])).expect("key found");
    assert_eq!(entry.chunk_indices, vec![0]);
    assert_eq!(entry.total_size, 6);

    // ChunkPool: read chunk 0 = the unique ciphertext.
    let pool = view.section(SectionId::ChunkPool).unwrap();
    assert_eq!(read_chunk(pool, 0).unwrap(), b"abcdef");

    // MerkleNodes decodes to the leaves, and CurrentRoot == tree root (D5).
    let mn = view.section(SectionId::MerkleNodes).unwrap();
    let leaves = decode_merkle_leaves(mn).unwrap();
    assert_eq!(leaves, inp.merkle_leaves);
    assert_eq!(MerkleTree::from_leaves(leaves).root(), inp.current_root);

    // Filler section present verbatim.
    assert_eq!(view.section(SectionId::Filler).unwrap(), &inp.filler[..]);
}

#[test]
fn data_section_matches_golden_vector() {
    let blob = encode_data_section(&fixed_inputs());
    let got = hex::encode(&blob);
    let expected = include_str!("fixtures/golden_data_section.hex").trim();
    if got != expected {
        eprintln!("GOLDEN MISMATCH. Review the structural test, then if intentional update the fixture to:\n{got}");
    }
    assert_eq!(
        got, expected,
        "data-section layout changed; structural test guards correctness"
    );
}
