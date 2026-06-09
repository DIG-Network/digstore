//! Canonical data-section format (BINDING contract D1/D3/D4/D5).
//!
//! This module is the **single source of truth** for the byte-exact data-section
//! blob that the compiler injects, the guest reads, and the client verifies.
//! It supersedes any per-crate data-section format.
//!
//! All multi-byte integers are **big-endian**.
//!
//! # Blob layout (D1)
//! ```text
//! magic   : b"DIGS"            (4 bytes)
//! version : u8 = 1             (1 byte)
//! count   : u32 BE             (4 bytes)   number of offset rows
//! rows    : count × 10 bytes   each = id:u16 BE | offset:u32 BE | len:u32 BE
//!                              (offset/len are relative to byte 0 of the blob)
//! bodies  : concatenated section bodies
//! ```
//! `total_len = max(offset+len)` — the blob is self-describing; no end symbol.

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::vec::Vec;

// Re-export the canonical `KeyTableEntry` so the contract API
// (`datasection::KeyTableEntry`) resolves to the single shared type.
pub use crate::keytable::KeyTableEntry;

/// Magic bytes at the start of a data-section blob.
pub const MAGIC: &[u8; 4] = b"DIGS";
/// Current data-section format version.
pub const VERSION: u8 = 1;
/// Fixed linear-memory offset where the compiler injects the blob and the guest
/// reads it: **2 MiB**, chosen to sit ABOVE the guest's static data (its rodata
/// lives at the wasm default global base of 1 MiB and ends well under 2 MiB) and
/// BELOW the guest's bump heap (which starts at 8 MiB) and the 16 MiB module cap.
///
/// BINDING contract D2 fix: the original 1 MiB value collided with the guest's
/// own static-data segment (the wasm linker places rodata at 1 MiB) — the
/// injected blob overwrote the guest's rodata, and the guest's heap (formerly an
/// 8 MiB static array starting just past 1 MiB) overlapped the injected chunk
/// pool, so a real compiled module dropped chunks past the first ~2 and did NOT
/// serve itself. The blob now occupies the dedicated window `[2 MiB, 8 MiB)`,
/// clear of both the guest's rodata below and its heap above.
pub const DIGS_DATA_OFFSET: u32 = 0x0020_0000;

/// Byte length of the fixed blob header (`magic` + `version` + `count`).
const HEADER_LEN: usize = 4 + 1 + 4;
/// Byte length of one offset-table row (`id:u16` + `offset:u32` + `len:u32`).
const ROW_LEN: usize = 2 + 4 + 4;

/// Logical section identifiers (`u16`, D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum SectionId {
    StoreId = 1,
    CurrentRoot = 2,
    RootHistory = 3,
    PublicKey = 4,
    TrustedKeys = 5,
    Metadata = 6,
    AuthInfo = 7,
    KeyTable = 8,
    ChunkPool = 9,
    MerkleNodes = 10,
    Filler = 11,
}

/// Build the data-section blob from `(id, body)` sections.
///
/// Bodies are laid out in the order given; the offset table records each
/// section's absolute offset (from byte 0 of the blob) and length.
pub fn encode_blob(sections: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let count = sections.len();
    let bodies_start = HEADER_LEN + count * ROW_LEN;

    // Header.
    let total_body: usize = sections.iter().map(|(_, b)| b.len()).sum();
    let mut out = Vec::with_capacity(bodies_start + total_body);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(count as u32).to_be_bytes());

    // Offset rows.
    let mut offset = bodies_start as u32;
    for (id, body) in sections {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        offset += body.len() as u32;
    }

    // Bodies.
    for (_, body) in sections {
        out.extend_from_slice(body);
    }
    out
}

/// One parsed offset-table row.
#[derive(Debug, Clone, Copy)]
struct Row {
    id: u16,
    offset: u32,
    len: u32,
}

/// Zero-copy reader over a parsed data-section blob.
#[derive(Debug, Clone)]
pub struct DataView<'a> {
    raw: &'a [u8],
    rows: Vec<Row>,
    total_len: usize,
}

impl<'a> DataView<'a> {
    /// Parse the header + offset table of a blob, validating magic, version, and
    /// that every row lies within `raw`.
    pub fn parse(raw: &'a [u8]) -> Result<DataView<'a>, DecodeError> {
        if raw.len() < HEADER_LEN {
            return Err(DecodeError::UnexpectedEof);
        }
        if &raw[0..4] != MAGIC {
            return Err(DecodeError::Invalid("bad DIGS magic"));
        }
        if raw[4] != VERSION {
            return Err(DecodeError::Invalid("unknown data-section version"));
        }
        let count = u32::from_be_bytes([raw[5], raw[6], raw[7], raw[8]]) as usize;

        let rows_end = HEADER_LEN
            .checked_add(
                count
                    .checked_mul(ROW_LEN)
                    .ok_or(DecodeError::UnexpectedEof)?,
            )
            .ok_or(DecodeError::UnexpectedEof)?;
        if raw.len() < rows_end {
            return Err(DecodeError::UnexpectedEof);
        }

        let mut rows = Vec::with_capacity(count);
        let mut total_len: usize = 0;
        for i in 0..count {
            let base = HEADER_LEN + i * ROW_LEN;
            let id = u16::from_be_bytes([raw[base], raw[base + 1]]);
            let offset =
                u32::from_be_bytes([raw[base + 2], raw[base + 3], raw[base + 4], raw[base + 5]]);
            let len =
                u32::from_be_bytes([raw[base + 6], raw[base + 7], raw[base + 8], raw[base + 9]]);
            // Every body must lie within the blob.
            let end = (offset as usize)
                .checked_add(len as usize)
                .ok_or(DecodeError::UnexpectedEof)?;
            if end > raw.len() {
                return Err(DecodeError::UnexpectedEof);
            }
            if end > total_len {
                total_len = end;
            }
            rows.push(Row { id, offset, len });
        }
        // If there are no rows, total_len is just the header+rows region.
        if count == 0 {
            total_len = rows_end;
        }

        Ok(DataView {
            raw,
            rows,
            total_len,
        })
    }

    /// Return the exact body slice for `id`, or `None` if absent.
    pub fn section(&self, id: SectionId) -> Option<&'a [u8]> {
        let target = id as u16;
        self.rows.iter().find(|r| r.id == target).map(|r| {
            let start = r.offset as usize;
            let end = start + r.len as usize;
            &self.raw[start..end]
        })
    }

    /// `max(offset + len)` over all rows — the self-describing total length.
    pub fn total_len(&self) -> usize {
        self.total_len
    }
}

// ---------------------------------------------------------------------------
// D3: KeyTable body (id 8)
//   count : u32 BE
//   per entry: static_key(32) | generation(32) | index_count u32 BE
//              | indices (index_count × u32 BE) | total_size u64 BE
// ---------------------------------------------------------------------------

/// Encode the KeyTable body from entries (D3).
pub fn encode_key_table(entries: &[KeyTableEntry]) -> Vec<u8> {
    let mut enc = Encoder::new();
    (entries.len() as u32).encode(&mut enc);
    for e in entries {
        e.encode(&mut enc);
    }
    enc.finish()
}

/// Find the entry whose `static_key` equals `retrieval_key`, scanning the body.
pub fn lookup_key(key_table_body: &[u8], retrieval_key: &Bytes32) -> Option<KeyTableEntry> {
    let mut dec = Decoder::new(key_table_body);
    let count = u32::decode(&mut dec).ok()?;
    for _ in 0..count {
        let entry = KeyTableEntry::decode(&mut dec).ok()?;
        if entry.static_key == *retrieval_key {
            return Some(entry);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// D4: ChunkPool body (id 9)
//   count : u32 BE
//   per chunk (GLOBAL INDEX ORDER): len u32 BE | bytes(ciphertext)
// ---------------------------------------------------------------------------

/// Encode the ChunkPool body from chunks in global-index order (D4).
pub fn encode_chunk_pool(chunks_in_global_index_order: &[&[u8]]) -> Vec<u8> {
    let total: usize = chunks_in_global_index_order
        .iter()
        .map(|c| 4 + c.len())
        .sum();
    let mut out = Vec::with_capacity(4 + total);
    out.extend_from_slice(&(chunks_in_global_index_order.len() as u32).to_be_bytes());
    for chunk in chunks_in_global_index_order {
        out.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        out.extend_from_slice(chunk);
    }
    out
}

/// Return the `global_index`-th chunk's ciphertext, or `None` if out of range
/// or the body is malformed.
pub fn read_chunk(pool_body: &[u8], global_index: u32) -> Option<&[u8]> {
    if pool_body.len() < 4 {
        return None;
    }
    let count = u32::from_be_bytes([pool_body[0], pool_body[1], pool_body[2], pool_body[3]]);
    if global_index >= count {
        return None;
    }
    let mut pos = 4usize;
    for i in 0..count {
        if pos + 4 > pool_body.len() {
            return None;
        }
        let len = u32::from_be_bytes([
            pool_body[pos],
            pool_body[pos + 1],
            pool_body[pos + 2],
            pool_body[pos + 3],
        ]) as usize;
        pos += 4;
        let end = pos.checked_add(len)?;
        if end > pool_body.len() {
            return None;
        }
        if i == global_index {
            return Some(&pool_body[pos..end]);
        }
        pos = end;
    }
    None
}

// ---------------------------------------------------------------------------
// D5: MerkleNodes body (id 10) = Vec<Bytes32>
//   count : u32 BE, then count × 32 raw bytes
// ---------------------------------------------------------------------------

/// Encode the MerkleNodes body: the ordered resource leaves (D5).
pub fn encode_merkle_nodes(leaves: &[Bytes32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + leaves.len() * 32);
    out.extend_from_slice(&(leaves.len() as u32).to_be_bytes());
    for leaf in leaves {
        out.extend_from_slice(&leaf.0);
    }
    out
}

/// Decode the MerkleNodes body into resource leaves (D5).
pub fn decode_merkle_leaves(body: &[u8]) -> Result<Vec<Bytes32>, DecodeError> {
    if body.len() < 4 {
        return Err(DecodeError::UnexpectedEof);
    }
    let count = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let needed = count
        .checked_mul(32)
        .and_then(|n| n.checked_add(4))
        .ok_or(DecodeError::UnexpectedEof)?;
    if body.len() < needed {
        return Err(DecodeError::UnexpectedEof);
    }
    let mut out = Vec::with_capacity(count);
    let mut pos = 4usize;
    for _ in 0..count {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&body[pos..pos + 32]);
        out.push(Bytes32(arr));
        pos += 32;
    }
    Ok(out)
}
