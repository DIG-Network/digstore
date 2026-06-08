//! Read-only view over the compiler-injected data section. The compiler writes
//! a known linear-memory region; the guest parses the `DIGS` header + offset
//! table. Big-endian throughout (DOC DEVIATION: Chia-compat over paper LE note).

use alloc::vec::Vec;
use digstore_core::Bytes32;
use digstore_core::KeyTableEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionError;

struct Entry {
    id: u16,
    off: usize,
    len: usize,
}

pub struct DataSection<'a> {
    raw: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> DataSection<'a> {
    pub fn parse(raw: &'a [u8]) -> Result<Self, SectionError> {
        if raw.len() < 9 || &raw[0..4] != b"DIGS" {
            return Err(SectionError);
        }
        if raw[4] != 1 {
            return Err(SectionError);
        }
        let count = u32::from_be_bytes([raw[5], raw[6], raw[7], raw[8]]) as usize;
        let table_start = 9usize;
        let table_end = table_start + count * 10;
        if raw.len() < table_end {
            return Err(SectionError);
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let p = table_start + i * 10;
            let id = u16::from_be_bytes([raw[p], raw[p + 1]]);
            let off = u32::from_be_bytes([raw[p + 2], raw[p + 3], raw[p + 4], raw[p + 5]]) as usize;
            let len = u32::from_be_bytes([raw[p + 6], raw[p + 7], raw[p + 8], raw[p + 9]]) as usize;
            if off.checked_add(len).is_none_or(|e| e > raw.len()) {
                return Err(SectionError);
            }
            entries.push(Entry { id, off, len });
        }
        Ok(DataSection { raw, entries })
    }

    pub fn section(&self, id: SectionId) -> Option<&'a [u8]> {
        let target = id as u16;
        self.entries
            .iter()
            .find(|e| e.id == target)
            .map(|e| &self.raw[e.off..e.off + e.len])
    }

    pub fn store_id(&self) -> Bytes32 {
        let s = self.section(SectionId::StoreId).unwrap_or(&[]);
        let mut a = [0u8; 32];
        a[..s.len().min(32)].copy_from_slice(&s[..s.len().min(32)]);
        Bytes32(a)
    }

    pub fn current_root(&self) -> Bytes32 {
        let s = self.section(SectionId::CurrentRoot).unwrap_or(&[]);
        let mut a = [0u8; 32];
        a[..s.len().min(32)].copy_from_slice(&s[..s.len().min(32)]);
        Bytes32(a)
    }

    /// Linear scan of the key table for a matching `static_key` (the retrieval key).
    /// Constant-shape: callers gate misses into the decoy path, so timing leaks
    /// nothing the oblivious layer does not already pad.
    pub fn lookup_key(&self, retrieval_key: &Bytes32) -> Option<KeyTableEntry> {
        let buf = self.section(SectionId::KeyTable)?;
        if buf.len() < 4 {
            return None;
        }
        let count = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let mut p = 4usize;
        for _ in 0..count {
            if p + 68 > buf.len() {
                return None;
            }
            let mut sk = [0u8; 32];
            sk.copy_from_slice(&buf[p..p + 32]);
            let mut gen = [0u8; 32];
            gen.copy_from_slice(&buf[p + 32..p + 64]);
            let icount =
                u32::from_be_bytes([buf[p + 64], buf[p + 65], buf[p + 66], buf[p + 67]]) as usize;
            p += 68;
            if p + icount * 4 + 8 > buf.len() {
                return None;
            }
            let mut indices = Vec::with_capacity(icount);
            for _ in 0..icount {
                indices.push(u32::from_be_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]]));
                p += 4;
            }
            let mut ts = [0u8; 8];
            ts.copy_from_slice(&buf[p..p + 8]);
            p += 8;
            if sk == retrieval_key.0 {
                return Some(KeyTableEntry {
                    static_key: Bytes32(sk),
                    generation: Bytes32(gen),
                    chunk_indices: indices,
                    total_size: u64::from_be_bytes(ts),
                });
            }
        }
        None
    }
}

/// Encode a key table: u32 BE count, then per entry:
/// static_key(32) | generation(32) | indices_count(u32 BE) | indices(u32 BE each) | total_size(u64 BE).
pub fn encode_key_table(entries: &[KeyTableEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for e in entries {
        out.extend_from_slice(&e.static_key.0);
        out.extend_from_slice(&e.generation.0);
        out.extend_from_slice(&(e.chunk_indices.len() as u32).to_be_bytes());
        for idx in &e.chunk_indices {
            out.extend_from_slice(&idx.to_be_bytes());
        }
        out.extend_from_slice(&e.total_size.to_be_bytes());
    }
    out
}

// The compiler injects the data section at a fixed symbol. The guest template
// reserves a static region the compiler overwrites with the real bytes.
#[cfg(target_arch = "wasm32")]
extern "C" {
    // Provided by a custom data segment the compiler injects, exported as linker
    // symbols `__digstore_data` (start) and `__digstore_data_end`.
    static __digstore_data: u8;
    static __digstore_data_end: u8;
}

#[cfg(target_arch = "wasm32")]
pub fn embedded<'a>() -> DataSection<'a> {
    unsafe {
        let start = core::ptr::addr_of!(__digstore_data);
        let end = core::ptr::addr_of!(__digstore_data_end);
        let len = end as usize - start as usize;
        let slice = core::slice::from_raw_parts(start, len);
        DataSection::parse(slice).unwrap_or(DataSection {
            raw: &[],
            entries: alloc::vec::Vec::new(),
        })
    }
}
