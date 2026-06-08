//! Shared fixture builders for synthetic DataSection blobs. Included as a module
//! by integration tests (`mod fixtures;`). Some helpers are only used by certain
//! test binaries, so silence dead-code warnings here.
#![allow(dead_code)]

use digstore_core::Bytes32;
use digstore_guest::datasection::{DataSection, SectionId};

/// Build a minimal valid data-section blob: magic `DIGS`, version 1, an offset
/// table for 3 sections (StoreId, RootHash, ChunkPool), then payloads.
pub fn build_minimal_section(store_id: [u8; 32], root: [u8; 32], pool: &[u8]) -> Vec<u8> {
    // header: magic(4) + version(1) + section_count(u32 BE)
    let mut header = Vec::new();
    header.extend_from_slice(b"DIGS");
    header.push(1u8);
    let count = 3u32;
    header.extend_from_slice(&count.to_be_bytes());

    // Each table entry: id(u16 BE) + offset(u32 BE) + len(u32 BE). Entry = 10 bytes.
    let table_size = (count as usize) * 10;
    let body_start = header.len() + table_size;

    let s0 = store_id.to_vec();
    let s1 = root.to_vec();
    let s2 = pool.to_vec();

    let off0 = body_start;
    let off1 = off0 + s0.len();
    let off2 = off1 + s1.len();

    let mut table = Vec::new();
    let push = |t: &mut Vec<u8>, id: u16, off: usize, len: usize| {
        t.extend_from_slice(&id.to_be_bytes());
        t.extend_from_slice(&(off as u32).to_be_bytes());
        t.extend_from_slice(&(len as u32).to_be_bytes());
    };
    push(&mut table, SectionId::StoreId as u16, off0, s0.len());
    push(&mut table, SectionId::CurrentRoot as u16, off1, s1.len());
    push(&mut table, SectionId::ChunkPool as u16, off2, s2.len());

    let mut out = header;
    out.extend_from_slice(&table);
    out.extend_from_slice(&s0);
    out.extend_from_slice(&s1);
    out.extend_from_slice(&s2);
    out
}

#[test]
fn parses_header_and_resolves_sections() {
    let blob = build_minimal_section([0xAA; 32], [0xBB; 32], &[1, 2, 3, 4]);
    let ds = DataSection::parse(&blob).expect("valid section");
    assert_eq!(ds.store_id(), Bytes32([0xAA; 32]));
    assert_eq!(ds.current_root(), Bytes32([0xBB; 32]));
    assert_eq!(ds.section(SectionId::ChunkPool), Some(&[1u8, 2, 3, 4][..]));
}

#[test]
fn rejects_bad_magic() {
    let mut blob = build_minimal_section([0; 32], [0; 32], &[]);
    blob[0] = b'X';
    assert!(DataSection::parse(&blob).is_err());
}

#[test]
fn rejects_bad_version() {
    let mut blob = build_minimal_section([0; 32], [0; 32], &[]);
    blob[4] = 2;
    assert!(DataSection::parse(&blob).is_err());
}

use digstore_core::KeyTableEntry;
use digstore_guest::datasection::encode_key_table;

#[test]
fn key_table_lookup_hit_and_miss() {
    let entry = KeyTableEntry {
        static_key: Bytes32([0x11; 32]),
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0, 2, 5],
        total_size: 4096,
    };
    let table_bytes = encode_key_table(&[entry.clone()]);
    let blob = build_section_with_keytable([0xAA; 32], [0xBB; 32], &table_bytes);
    let ds = DataSection::parse(&blob).unwrap();

    let hit = ds.lookup_key(&Bytes32([0x11; 32])).expect("entry present");
    assert_eq!(hit.chunk_indices, vec![0, 2, 5]);
    assert_eq!(hit.total_size, 4096);

    assert!(ds.lookup_key(&Bytes32([0x99; 32])).is_none(), "miss returns None");
}

/// Variant of the fixture that places a KeyTable section instead of a ChunkPool.
pub fn build_section_with_keytable(store_id: [u8; 32], root: [u8; 32], table: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(b"DIGS");
    header.push(1u8);
    header.extend_from_slice(&3u32.to_be_bytes());
    let body_start = header.len() + 30;
    let s0 = store_id.to_vec();
    let s1 = root.to_vec();
    let s2 = table.to_vec();
    let off0 = body_start;
    let off1 = off0 + s0.len();
    let off2 = off1 + s1.len();
    let mut table_bytes = Vec::new();
    let push = |t: &mut Vec<u8>, id: u16, off: usize, len: usize| {
        t.extend_from_slice(&id.to_be_bytes());
        t.extend_from_slice(&(off as u32).to_be_bytes());
        t.extend_from_slice(&(len as u32).to_be_bytes());
    };
    push(&mut table_bytes, SectionId::StoreId as u16, off0, s0.len());
    push(&mut table_bytes, SectionId::CurrentRoot as u16, off1, s1.len());
    push(&mut table_bytes, SectionId::KeyTable as u16, off2, s2.len());
    let mut out = header;
    out.extend_from_slice(&table_bytes);
    out.extend_from_slice(&s0);
    out.extend_from_slice(&s1);
    out.extend_from_slice(&s2);
    out
}

/// Pack a chunk pool: count(u32 BE) then per chunk: len(u32 BE) || bytes.
pub fn pack_pool(chunks: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(chunks.len() as u32).to_be_bytes());
    for c in chunks {
        out.extend_from_slice(&(c.len() as u32).to_be_bytes());
        out.extend_from_slice(c);
    }
    out
}

/// Section blob carrying StoreId, CurrentRoot, KeyTable, ChunkPool (4 sections).
pub fn section_keytable_and_pool(
    store_id: [u8; 32],
    root: [u8; 32],
    table: &[u8],
    pool: &[u8],
) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(b"DIGS");
    header.push(1u8);
    header.extend_from_slice(&4u32.to_be_bytes());
    let body_start = header.len() + 40;
    let parts: [(&[u8], u16); 4] = [
        (&store_id[..], SectionId::StoreId as u16),
        (&root[..], SectionId::CurrentRoot as u16),
        (table, SectionId::KeyTable as u16),
        (pool, SectionId::ChunkPool as u16),
    ];
    let mut table_bytes = Vec::new();
    let mut off = body_start;
    for (data, id) in &parts {
        table_bytes.extend_from_slice(&id.to_be_bytes());
        table_bytes.extend_from_slice(&(off as u32).to_be_bytes());
        table_bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        off += data.len();
    }
    let mut out = header;
    out.extend_from_slice(&table_bytes);
    for (data, _) in &parts {
        out.extend_from_slice(data);
    }
    out
}

pub fn section_with_metadata(store_id: [u8; 32], root: [u8; 32], manifest: &[u8]) -> Vec<u8> {
    build_three(store_id, root, manifest, SectionId::Metadata as u16)
}
pub fn section_with_pubkey(store_id: [u8; 32], root: [u8; 32], pk: &[u8]) -> Vec<u8> {
    build_three(store_id, root, pk, SectionId::PublicKey as u16)
}
fn build_three(store_id: [u8; 32], root: [u8; 32], third: &[u8], third_id: u16) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(b"DIGS");
    header.push(1u8);
    header.extend_from_slice(&3u32.to_be_bytes());
    let body_start = header.len() + 30;
    let parts: [(&[u8], u16); 3] = [
        (&store_id[..], SectionId::StoreId as u16),
        (&root[..], SectionId::CurrentRoot as u16),
        (third, third_id),
    ];
    let mut tbl = Vec::new();
    let mut off = body_start;
    for (d, id) in &parts {
        tbl.extend_from_slice(&id.to_be_bytes());
        tbl.extend_from_slice(&(off as u32).to_be_bytes());
        tbl.extend_from_slice(&(d.len() as u32).to_be_bytes());
        off += d.len();
    }
    let mut out = header;
    out.extend_from_slice(&tbl);
    for (d, _) in &parts {
        out.extend_from_slice(d);
    }
    out
}
