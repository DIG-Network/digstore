mod common;

use digstore_compiler::{
    decode_store_header, decode_trusted_keys, encode_data_section, parse_offset_table, ChunkLoc,
    DataSectionInputs, SEG_KEY_TABLE, SEG_MANIFEST, SEG_POOL, SEG_STORE_HEADER, SEG_TRUSTED_KEYS,
};
use digstore_core::{Bytes32, Bytes48, Decode, Decoder, KeyTableEntry, MetadataManifest, TrustedHostKey};

fn fixed_inputs() -> DataSectionInputs {
    DataSectionInputs {
        store_id: Bytes32([0xAB; 32]),
        roothash: Bytes32([0x11; 32]),
        root_history: vec![Bytes32([0x11; 32])],
        store_pubkey: Bytes48([0xCD; 48]),
        pool_bytes: vec![0x09; 16],
        pool_descriptors: vec![ChunkLoc { offset: 0, len: 16 }],
        key_table: vec![KeyTableEntry {
            static_key: Bytes32([0x01; 32]),
            generation: Bytes32([0x11; 32]),
            chunk_indices: vec![0],
            total_size: 16,
        }],
        manifest: common::sample_manifest(),
        trusted_keys: vec![TrustedHostKey {
            public_key: [0x42u8; 48],
            label: "L".into(),
        }],
    }
}

#[test]
fn structure_is_independently_valid() {
    let inp = fixed_inputs();
    let blob = encode_data_section(&inp);

    // Header.
    assert_eq!(&blob[0..4], b"DIGS");
    assert_eq!(blob[4], 1u8);

    // Offset table: five canonical segments, ascending, in bounds.
    let table = parse_offset_table(&blob).expect("table parses");
    let kinds: Vec<u8> = table.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![SEG_POOL, SEG_KEY_TABLE, SEG_STORE_HEADER, SEG_MANIFEST, SEG_TRUSTED_KEYS]
    );

    // Pool segment decodes to the original bytes + descriptors.
    let pool_seg = table.iter().find(|e| e.kind == SEG_POOL).unwrap();
    let pool_body = &blob[pool_seg.offset as usize..(pool_seg.offset + pool_seg.len) as usize];
    let mut dec = Decoder::new(pool_body);
    let pool_bytes = Vec::<u8>::decode(&mut dec).unwrap();
    let descs = Vec::<ChunkLoc>::decode(&mut dec).unwrap();
    assert_eq!(pool_bytes, vec![0x09u8; 16]);
    assert_eq!(descs, vec![ChunkLoc { offset: 0, len: 16 }]);

    // Key table decodes.
    let kt_seg = table.iter().find(|e| e.kind == SEG_KEY_TABLE).unwrap();
    let kt_body = &blob[kt_seg.offset as usize..(kt_seg.offset + kt_seg.len) as usize];
    let mut dec = Decoder::new(kt_body);
    let entries = Vec::<KeyTableEntry>::decode(&mut dec).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].chunk_indices, vec![0]);
    assert_eq!(entries[0].total_size, 16);

    // Store header decodes to the original identity fields.
    let sh_seg = table.iter().find(|e| e.kind == SEG_STORE_HEADER).unwrap();
    let sh_body = &blob[sh_seg.offset as usize..(sh_seg.offset + sh_seg.len) as usize];
    let (sid, root, hist, pk) = decode_store_header(sh_body).unwrap();
    assert_eq!(sid, inp.store_id);
    assert_eq!(root, inp.roothash);
    assert_eq!(hist, inp.root_history);
    assert_eq!(pk, inp.store_pubkey);

    // Manifest decodes.
    let mf_seg = table.iter().find(|e| e.kind == SEG_MANIFEST).unwrap();
    let mf_body = &blob[mf_seg.offset as usize..(mf_seg.offset + mf_seg.len) as usize];
    let mut dec = Decoder::new(mf_body);
    let mf = MetadataManifest::decode(&mut dec).unwrap();
    assert_eq!(mf.name, "sample-store");

    // Trusted keys decode.
    let tk_seg = table.iter().find(|e| e.kind == SEG_TRUSTED_KEYS).unwrap();
    let tk_body = &blob[tk_seg.offset as usize..(tk_seg.offset + tk_seg.len) as usize];
    let keys = decode_trusted_keys(tk_body).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].label, "L");
}

#[test]
fn data_section_matches_golden_vector() {
    let blob = encode_data_section(&fixed_inputs());
    let got = hex::encode(&blob);
    let expected = include_str!("fixtures/golden_data_section.hex").trim();
    if got != expected {
        eprintln!("GOLDEN MISMATCH. Review the structural test, then if intentional update the fixture to:\n{got}");
    }
    assert_eq!(got, expected, "data-section layout changed; structural test guards correctness");
}
