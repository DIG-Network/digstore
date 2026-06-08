//! Encode/parse the full data-section blob.
//!
//! Layout (locked): magic `b"DIGS"`, `u8 format_version = 1`, then a section
//! offset table (4-byte BE count, then rows of `kind:u8, offset:u32 BE,
//! len:u32 BE`), then the concatenated segment bodies. The outer header/offset
//! framing is LOCAL to the compiler (the guest's first read parses it). Every
//! typed segment body is produced by `digstore_core`'s canonical big-endian
//! `Encode` so the bytes are byte-identical to what the guest's `Decode`
//! consumes. Multi-byte integers are big-endian (Chia streamable, deviation #1).

use digstore_core::{
    Bytes32, Bytes48, Decode, Decoder, Encode, Encoder, KeyTableEntry, MetadataManifest,
    TrustedHostKey,
};

use crate::error::{CompilerError, Result};
use crate::pool::ChunkLoc;

pub const MAGIC: &[u8; 4] = b"DIGS";
pub const FORMAT_VERSION: u8 = 1;

pub const SEG_POOL: u8 = 0;
pub const SEG_KEY_TABLE: u8 = 1;
pub const SEG_STORE_HEADER: u8 = 2;
pub const SEG_MANIFEST: u8 = 3;
pub const SEG_TRUSTED_KEYS: u8 = 4;

/// One row of the section offset table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionEntry {
    pub kind: u8,
    pub offset: u32,
    pub len: u32,
}

/// All inputs needed to encode the data section (gathered by the pipeline). All
/// 48-byte keys use the canonical `Bytes48` newtype; `TrustedHostKey` carries its
/// own `[u8;48]` per the catalog and is encoded via core `Encode`.
pub struct DataSectionInputs {
    pub store_id: Bytes32,
    pub roothash: Bytes32,
    pub root_history: Vec<Bytes32>,
    pub store_pubkey: Bytes48,
    pub pool_bytes: Vec<u8>,
    pub pool_descriptors: Vec<ChunkLoc>,
    pub key_table: Vec<KeyTableEntry>,
    pub manifest: MetadataManifest,
    pub trusted_keys: Vec<TrustedHostKey>,
}

// ---- segment body encoders (ALL via canonical core Encode) ----

/// Encode raw bytes with a 4-byte BE length prefix (matches the core `Vec<u8>`
/// framing the guest's `Vec::<u8>::decode` expects).
fn encode_byte_blob(bytes: &[u8], enc: &mut Encoder) {
    (bytes.len() as u32).encode(enc);
    enc.write_bytes(bytes);
}

fn encode_pool_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut enc = Encoder::new();
    encode_byte_blob(&i.pool_bytes, &mut enc); // BE u32 len + raw bytes
    i.pool_descriptors.encode(&mut enc); // Vec<ChunkLoc>: BE u32 count + items
    enc.finish()
}

fn encode_key_table_segment(i: &DataSectionInputs) -> Vec<u8> {
    i.key_table.to_bytes()
}

fn encode_store_header_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut enc = Encoder::new();
    i.store_id.encode(&mut enc);
    i.roothash.encode(&mut enc);
    i.root_history.encode(&mut enc);
    i.store_pubkey.encode(&mut enc);
    enc.finish()
}

fn encode_manifest_segment(i: &DataSectionInputs) -> Vec<u8> {
    i.manifest.to_bytes()
}

/// Encode `Vec<TrustedHostKey>` using core primitive framing field-by-field.
///
/// DEVIATION: `digstore_core::TrustedHostKey` does NOT implement `Encode`/`Decode`
/// (the compiler may not edit core). We reproduce the exact framing core would use
/// for a derived impl — `Vec` is a 4-byte BE count, each entry is
/// `[u8;48] public_key` (raw, no prefix) then `String label` (4-byte BE len +
/// bytes) — so the guest's matching decode reads identical bytes.
fn encode_trusted_keys_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut enc = Encoder::new();
    (i.trusted_keys.len() as u32).encode(&mut enc);
    for k in &i.trusted_keys {
        k.public_key.encode(&mut enc);
        k.label.encode(&mut enc);
    }
    enc.finish()
}

/// Decode `Vec<TrustedHostKey>` produced by [`encode_trusted_keys_segment`].
pub fn decode_trusted_keys(body: &[u8]) -> Result<Vec<TrustedHostKey>> {
    let mut dec = Decoder::new(body);
    let count = u32::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))? as usize;
    let mut out = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let public_key =
            <[u8; 48]>::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
        let label =
            String::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
        out.push(TrustedHostKey { public_key, label });
    }
    Ok(out)
}

/// Encode the full data-section blob: magic + version + offset table + segments.
pub fn encode_data_section(i: &DataSectionInputs) -> Vec<u8> {
    let segments: Vec<(u8, Vec<u8>)> = vec![
        (SEG_POOL, encode_pool_segment(i)),
        (SEG_KEY_TABLE, encode_key_table_segment(i)),
        (SEG_STORE_HEADER, encode_store_header_segment(i)),
        (SEG_MANIFEST, encode_manifest_segment(i)),
        (SEG_TRUSTED_KEYS, encode_trusted_keys_segment(i)),
    ];

    // Header = magic(4) + version(1) + count(4) + table(count * (1+4+4)).
    let header_len = 4 + 1 + 4 + segments.len() * (1 + 4 + 4);
    let mut offset = header_len as u32;

    let mut table = Vec::with_capacity(segments.len());
    for (kind, body) in &segments {
        table.push(SectionEntry {
            kind: *kind,
            offset,
            len: body.len() as u32,
        });
        offset += body.len() as u32;
    }

    let mut out = Vec::with_capacity(offset as usize);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&(segments.len() as u32).to_be_bytes());
    for e in &table {
        out.push(e.kind);
        out.extend_from_slice(&e.offset.to_be_bytes());
        out.extend_from_slice(&e.len.to_be_bytes());
    }
    for (_kind, body) in &segments {
        out.extend_from_slice(body);
    }
    debug_assert_eq!(out.len(), offset as usize);
    out
}

/// Parse the offset table. Returns `CompilerError::Validation` on malformed input
/// (bad magic/version, truncated table) rather than panicking.
pub fn parse_offset_table(blob: &[u8]) -> Result<Vec<SectionEntry>> {
    if blob.len() < 9 {
        return Err(CompilerError::Validation("data section too short".into()));
    }
    if &blob[0..4] != MAGIC {
        return Err(CompilerError::Validation("bad data-section magic".into()));
    }
    if blob[4] != FORMAT_VERSION {
        return Err(CompilerError::Validation(format!(
            "unsupported data-section version {}",
            blob[4]
        )));
    }
    let count = u32::from_be_bytes([blob[5], blob[6], blob[7], blob[8]]) as usize;
    let table_end = 9 + count * 9;
    if blob.len() < table_end {
        return Err(CompilerError::Validation("offset table truncated".into()));
    }
    let mut entries = Vec::with_capacity(count);
    let mut p = 9usize;
    for _ in 0..count {
        let kind = blob[p];
        let offset = u32::from_be_bytes([blob[p + 1], blob[p + 2], blob[p + 3], blob[p + 4]]);
        let len = u32::from_be_bytes([blob[p + 5], blob[p + 6], blob[p + 7], blob[p + 8]]);
        if (offset as usize)
            .checked_add(len as usize)
            .map(|end| end > blob.len())
            .unwrap_or(true)
        {
            return Err(CompilerError::Validation("segment out of bounds".into()));
        }
        entries.push(SectionEntry { kind, offset, len });
        p += 9;
    }
    Ok(entries)
}

/// Decode the store-header segment back to its fields (used by the round-trip test
/// and mirrors the guest's decode path).
pub fn decode_store_header(body: &[u8]) -> Result<(Bytes32, Bytes32, Vec<Bytes32>, Bytes48)> {
    let mut dec = Decoder::new(body);
    let store_id = Bytes32::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
    let roothash = Bytes32::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
    let root_history =
        Vec::<Bytes32>::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
    let pubkey = Bytes48::decode(&mut dec).map_err(|e| CompilerError::Validation(format!("{e:?}")))?;
    Ok((store_id, roothash, root_history, pubkey))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::ChunkLoc;
    use digstore_core::{Bytes32, Bytes48, KeyTableEntry, MetadataManifest, TrustedHostKey};

    fn manifest() -> MetadataManifest {
        MetadataManifest {
            schema_version: 1,
            name: "n".into(),
            version: None,
            description: None,
            authors: vec![],
            license: None,
            homepage: None,
            repository: None,
            keywords: vec![],
            categories: vec![],
            icon: None,
            content_type: None,
            links: Default::default(),
            custom: Default::default(),
        }
    }

    fn inputs() -> DataSectionInputs {
        DataSectionInputs {
            store_id: Bytes32([0xAB; 32]),
            roothash: Bytes32([0x11; 32]),
            root_history: vec![Bytes32([0x11; 32])],
            store_pubkey: Bytes48([0xCD; 48]),
            pool_bytes: vec![9u8; 64],
            pool_descriptors: vec![ChunkLoc { offset: 0, len: 64 }],
            key_table: vec![KeyTableEntry {
                static_key: Bytes32([1; 32]),
                generation: Bytes32([0x11; 32]),
                chunk_indices: vec![0],
                total_size: 64,
            }],
            manifest: manifest(),
            trusted_keys: vec![TrustedHostKey {
                public_key: [0x42u8; 48],
                label: "dig-host-key-v1:abc".into(),
            }],
        }
    }

    #[test]
    fn starts_with_magic_and_version() {
        let blob = encode_data_section(&inputs());
        assert_eq!(&blob[0..4], b"DIGS");
        assert_eq!(blob[4], 1u8);
    }

    #[test]
    fn offset_table_has_five_segments_big_endian_count() {
        let blob = encode_data_section(&inputs());
        let count = u32::from_be_bytes([blob[5], blob[6], blob[7], blob[8]]);
        assert_eq!(count, 5);
    }

    #[test]
    fn segment_offsets_are_ascending_in_bounds_and_canonical_order() {
        let blob = encode_data_section(&inputs());
        let table = parse_offset_table(&blob).expect("table parses");
        assert_eq!(table.len(), 5);
        let kinds: Vec<u8> = table.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![0, 1, 2, 3, 4]);
        let mut prev_end = 0u32;
        for e in &table {
            assert!(e.offset >= prev_end);
            assert!((e.offset + e.len) as usize <= blob.len());
            prev_end = e.offset + e.len;
        }
    }

    #[test]
    fn parse_offset_table_rejects_bad_magic() {
        let mut blob = encode_data_section(&inputs());
        blob[0] = b'X';
        assert!(parse_offset_table(&blob).is_err());
    }

    #[test]
    fn store_header_segment_round_trips_via_decode() {
        let inp = inputs();
        let blob = encode_data_section(&inp);
        let table = parse_offset_table(&blob).unwrap();
        let seg = table.iter().find(|e| e.kind == SEG_STORE_HEADER).unwrap();
        let body = &blob[seg.offset as usize..(seg.offset + seg.len) as usize];
        let (sid, root, hist, pk) = decode_store_header(body).unwrap();
        assert_eq!(sid, inp.store_id);
        assert_eq!(root, inp.roothash);
        assert_eq!(hist, inp.root_history);
        assert_eq!(pk, inp.store_pubkey);
    }
}
