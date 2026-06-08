//! Tests for the canonical `digstore_core::datasection` module (D1/D3/D4/D5 of
//! the BINDING data-section contract). All multi-byte fields are big-endian.

use digstore_core::datasection::{
    decode_merkle_leaves, encode_blob, encode_chunk_pool, encode_key_table, encode_merkle_nodes,
    lookup_key, read_chunk, DataView, KeyTableEntry, SectionId, DIGS_DATA_OFFSET, MAGIC, VERSION,
};
use digstore_core::Bytes32;

// ---------------------------------------------------------------------------
// D1: constants + blob layout
// ---------------------------------------------------------------------------

#[test]
fn constants_match_contract() {
    assert_eq!(MAGIC, b"DIGS");
    assert_eq!(VERSION, 1);
    assert_eq!(DIGS_DATA_OFFSET, 0x0010_0000);
    // Section IDs 1..=11.
    assert_eq!(SectionId::StoreId as u16, 1);
    assert_eq!(SectionId::CurrentRoot as u16, 2);
    assert_eq!(SectionId::RootHistory as u16, 3);
    assert_eq!(SectionId::PublicKey as u16, 4);
    assert_eq!(SectionId::TrustedKeys as u16, 5);
    assert_eq!(SectionId::Metadata as u16, 6);
    assert_eq!(SectionId::AuthInfo as u16, 7);
    assert_eq!(SectionId::KeyTable as u16, 8);
    assert_eq!(SectionId::ChunkPool as u16, 9);
    assert_eq!(SectionId::MerkleNodes as u16, 10);
    assert_eq!(SectionId::Filler as u16, 11);
}

#[test]
fn encode_blob_byte_exact_header_layout() {
    // Two sections: id=1 (StoreId, 32 bytes of 0xAA), id=2 (CurrentRoot, 4 bytes).
    let store = vec![0xAAu8; 32];
    let root = vec![1u8, 2, 3, 4];
    let blob = encode_blob(&[(1u16, store.clone()), (2u16, root.clone())]);

    // magic (4) | version (1) | count u32 BE (4) | 2 rows * 10 | bodies.
    assert_eq!(&blob[0..4], b"DIGS");
    assert_eq!(blob[4], 1u8); // version
    assert_eq!(&blob[5..9], &[0, 0, 0, 2]); // count = 2, BE

    let header_len = 4 + 1 + 4; // 9
    let rows_len = 2 * 10; // 20
    let bodies_start = header_len + rows_len; // 29

    // Row 0: id=1, off=29, len=32
    let r0 = &blob[header_len..header_len + 10];
    assert_eq!(&r0[0..2], &[0, 1]); // id u16 BE
    assert_eq!(&r0[2..6], &(bodies_start as u32).to_be_bytes()); // off u32 BE
    assert_eq!(&r0[6..10], &32u32.to_be_bytes()); // len u32 BE

    // Row 1: id=2, off=29+32=61, len=4
    let r1 = &blob[header_len + 10..header_len + 20];
    assert_eq!(&r1[0..2], &[0, 2]);
    assert_eq!(&r1[2..6], &((bodies_start + 32) as u32).to_be_bytes());
    assert_eq!(&r1[6..10], &4u32.to_be_bytes());

    // Bodies are concatenated in row order.
    assert_eq!(&blob[bodies_start..bodies_start + 32], &store[..]);
    assert_eq!(
        &blob[bodies_start + 32..bodies_start + 36],
        &root[..]
    );
    assert_eq!(blob.len(), bodies_start + 32 + 4);
}

#[test]
fn dataview_roundtrip_exact_slices() {
    let store = vec![0xAAu8; 32];
    let root = vec![0xBBu8; 32];
    let pubkey = vec![0xCCu8; 48];
    let blob = encode_blob(&[
        (SectionId::StoreId as u16, store.clone()),
        (SectionId::CurrentRoot as u16, root.clone()),
        (SectionId::PublicKey as u16, pubkey.clone()),
    ]);

    let view = DataView::parse(&blob).unwrap();
    assert_eq!(view.section(SectionId::StoreId), Some(&store[..]));
    assert_eq!(view.section(SectionId::CurrentRoot), Some(&root[..]));
    assert_eq!(view.section(SectionId::PublicKey), Some(&pubkey[..]));
    // A section not present returns None.
    assert_eq!(view.section(SectionId::ChunkPool), None);
    // total_len = max(offset + len) = full blob length here.
    assert_eq!(view.total_len(), blob.len());
}

#[test]
fn dataview_total_len_is_max_offset_plus_len() {
    // total_len must equal the largest (offset+len) over all rows, which for a
    // contiguous blob equals the blob length.
    let blob = encode_blob(&[
        (1u16, vec![9u8; 10]),
        (2u16, vec![8u8; 20]),
        (3u16, vec![7u8; 5]),
    ]);
    let view = DataView::parse(&blob).unwrap();
    assert_eq!(view.total_len(), blob.len());
}

#[test]
fn dataview_empty_section_body() {
    let blob = encode_blob(&[(1u16, vec![]), (2u16, vec![1, 2, 3])]);
    let view = DataView::parse(&blob).unwrap();
    assert_eq!(view.section(SectionId::StoreId), Some(&[][..]));
    assert_eq!(view.section(SectionId::CurrentRoot), Some(&[1u8, 2, 3][..]));
}

#[test]
fn parse_rejects_bad_magic() {
    let mut blob = encode_blob(&[(1u16, vec![1, 2, 3])]);
    blob[0] = b'X';
    assert!(DataView::parse(&blob).is_err());
}

#[test]
fn parse_rejects_bad_version() {
    let mut blob = encode_blob(&[(1u16, vec![1, 2, 3])]);
    blob[4] = 2; // version byte
    assert!(DataView::parse(&blob).is_err());
}

#[test]
fn parse_rejects_truncation() {
    let blob = encode_blob(&[(1u16, vec![0xAA; 32]), (2u16, vec![0xBB; 16])]);
    // Truncate inside the bodies.
    let truncated = &blob[..blob.len() - 4];
    assert!(DataView::parse(truncated).is_err());
    // Truncate inside the header rows.
    let truncated2 = &blob[..10];
    assert!(DataView::parse(truncated2).is_err());
    // Empty input.
    assert!(DataView::parse(&[]).is_err());
}

// ---------------------------------------------------------------------------
// D3: KeyTable
// ---------------------------------------------------------------------------

#[test]
fn key_table_roundtrip_and_lookup() {
    let e0 = KeyTableEntry {
        static_key: Bytes32([1u8; 32]),
        generation: Bytes32([9u8; 32]),
        chunk_indices: vec![0, 1, 2],
        total_size: 300,
    };
    let e1 = KeyTableEntry {
        static_key: Bytes32([2u8; 32]),
        generation: Bytes32([9u8; 32]),
        chunk_indices: vec![3],
        total_size: 100,
    };
    let body = encode_key_table(&[e0.clone(), e1.clone()]);

    // count u32 BE = 2
    assert_eq!(&body[0..4], &[0, 0, 0, 2]);

    // lookup finds both by static_key.
    let f0 = lookup_key(&body, &Bytes32([1u8; 32])).unwrap();
    assert_eq!(f0, e0);
    let f1 = lookup_key(&body, &Bytes32([2u8; 32])).unwrap();
    assert_eq!(f1, e1);

    // miss
    assert!(lookup_key(&body, &Bytes32([7u8; 32])).is_none());
}

#[test]
fn key_table_entry_byte_layout() {
    let e = KeyTableEntry {
        static_key: Bytes32([0u8; 32]),
        generation: Bytes32([0u8; 32]),
        chunk_indices: vec![0x01020304],
        total_size: 0xAABBCCDD,
    };
    let body = encode_key_table(&[e]);
    // count(4) + [static(32) + gen(32) + idx_count u32(4) + 1*u32(4) + total u64(8)]
    let expected_len = 4 + 32 + 32 + 4 + 4 + 8;
    assert_eq!(body.len(), expected_len);
    // index_count = 1 (BE) at offset 4+32+32
    let ic = &body[4 + 64..4 + 64 + 4];
    assert_eq!(ic, &1u32.to_be_bytes());
    // the index value
    let iv = &body[4 + 64 + 4..4 + 64 + 8];
    assert_eq!(iv, &0x01020304u32.to_be_bytes());
    // total_size u64 BE
    let ts = &body[4 + 64 + 8..4 + 64 + 16];
    assert_eq!(ts, &0xAABBCCDDu64.to_be_bytes());
}

// ---------------------------------------------------------------------------
// D4: ChunkPool
// ---------------------------------------------------------------------------

#[test]
fn chunk_pool_roundtrip_by_global_index() {
    let c0: &[u8] = b"hello";
    let c1: &[u8] = b"worldwide";
    let c2: &[u8] = b"!";
    let pool = encode_chunk_pool(&[c0, c1, c2]);

    // count u32 BE = 3
    assert_eq!(&pool[0..4], &[0, 0, 0, 3]);

    assert_eq!(read_chunk(&pool, 0), Some(c0));
    assert_eq!(read_chunk(&pool, 1), Some(c1));
    assert_eq!(read_chunk(&pool, 2), Some(c2));
    assert_eq!(read_chunk(&pool, 3), None); // out of range
}

#[test]
fn chunk_pool_byte_layout() {
    let pool = encode_chunk_pool(&[&b"ab"[..], &b"cde"[..]]);
    // count(4) | len(4)=2 | "ab" | len(4)=3 | "cde"
    assert_eq!(&pool[0..4], &2u32.to_be_bytes());
    assert_eq!(&pool[4..8], &2u32.to_be_bytes());
    assert_eq!(&pool[8..10], b"ab");
    assert_eq!(&pool[10..14], &3u32.to_be_bytes());
    assert_eq!(&pool[14..17], b"cde");
    assert_eq!(pool.len(), 17);
}

#[test]
fn chunk_pool_empty_chunk() {
    let pool = encode_chunk_pool(&[&b""[..], &b"x"[..]]);
    assert_eq!(read_chunk(&pool, 0), Some(&b""[..]));
    assert_eq!(read_chunk(&pool, 1), Some(&b"x"[..]));
}

// ---------------------------------------------------------------------------
// D5: MerkleNodes
// ---------------------------------------------------------------------------

#[test]
fn merkle_nodes_roundtrip() {
    let leaves = vec![Bytes32([1u8; 32]), Bytes32([2u8; 32]), Bytes32([3u8; 32])];
    let body = encode_merkle_nodes(&leaves);
    // count u32 BE = 3, then 3*32 raw
    assert_eq!(&body[0..4], &3u32.to_be_bytes());
    assert_eq!(body.len(), 4 + 3 * 32);
    let decoded = decode_merkle_leaves(&body).unwrap();
    assert_eq!(decoded, leaves);
}

#[test]
fn merkle_nodes_empty() {
    let body = encode_merkle_nodes(&[]);
    assert_eq!(&body[0..4], &0u32.to_be_bytes());
    assert_eq!(decode_merkle_leaves(&body).unwrap(), Vec::<Bytes32>::new());
}

#[test]
fn merkle_nodes_rejects_truncation() {
    let leaves = vec![Bytes32([1u8; 32]), Bytes32([2u8; 32])];
    let body = encode_merkle_nodes(&leaves);
    assert!(decode_merkle_leaves(&body[..body.len() - 1]).is_err());
    assert!(decode_merkle_leaves(&[0, 0]).is_err());
}

// ---------------------------------------------------------------------------
// Integration: a realistic self-describing blob with many sections.
// ---------------------------------------------------------------------------

#[test]
fn full_blob_with_many_sections() {
    let store_id = Bytes32([0x11u8; 32]);
    let current_root = Bytes32([0x22u8; 32]);
    let pubkey = vec![0x33u8; 48];

    let kt = encode_key_table(&[KeyTableEntry {
        static_key: Bytes32([0x44u8; 32]),
        generation: current_root,
        chunk_indices: vec![0, 1],
        total_size: 11,
    }]);
    let pool = encode_chunk_pool(&[&b"hello"[..], &b"world!"[..]]);
    let nodes = encode_merkle_nodes(&[Bytes32([0x55u8; 32])]);

    let blob = encode_blob(&[
        (SectionId::StoreId as u16, store_id.0.to_vec()),
        (SectionId::CurrentRoot as u16, current_root.0.to_vec()),
        (SectionId::PublicKey as u16, pubkey.clone()),
        (SectionId::KeyTable as u16, kt.clone()),
        (SectionId::ChunkPool as u16, pool.clone()),
        (SectionId::MerkleNodes as u16, nodes.clone()),
    ]);

    let view = DataView::parse(&blob).unwrap();
    assert_eq!(view.section(SectionId::StoreId), Some(&store_id.0[..]));
    assert_eq!(view.section(SectionId::CurrentRoot), Some(&current_root.0[..]));
    assert_eq!(view.section(SectionId::PublicKey), Some(&pubkey[..]));
    assert_eq!(view.total_len(), blob.len());

    // Drill into the KeyTable section via lookup_key.
    let kt_body = view.section(SectionId::KeyTable).unwrap();
    let entry = lookup_key(kt_body, &Bytes32([0x44u8; 32])).unwrap();
    assert_eq!(entry.chunk_indices, vec![0, 1]);

    // Drill into the ChunkPool section and read chunks by global index.
    let pool_body = view.section(SectionId::ChunkPool).unwrap();
    assert_eq!(read_chunk(pool_body, 0), Some(&b"hello"[..]));
    assert_eq!(read_chunk(pool_body, 1), Some(&b"world!"[..]));

    // Drill into the MerkleNodes section.
    let nodes_body = view.section(SectionId::MerkleNodes).unwrap();
    assert_eq!(
        decode_merkle_leaves(nodes_body).unwrap(),
        vec![Bytes32([0x55u8; 32])]
    );
}
