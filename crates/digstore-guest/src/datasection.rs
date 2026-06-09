//! Read-only view over the compiler-injected data section.
//!
//! BINDING contract D1/D2: the byte-exact data-section format is owned by
//! `digstore_core::datasection`. The guest does **not** define its own parser or
//! encoder — it delegates to core everywhere. This module is a thin guest-side
//! adapter that:
//!   * re-exports the canonical [`SectionId`] and [`encode_key_table`] from core,
//!   * wraps `digstore_core::datasection::DataView` in [`DataSection`] with the
//!     small convenience accessors (`store_id`, `current_root`, `lookup_key`,
//!     `section`) the guest's pure-logic modules call, and
//!   * reads the blob from the fixed pointer
//!     [`digstore_core::datasection::DIGS_DATA_OFFSET`] (D2) on wasm.
//!
//! The previous private 10-byte/`id:u16` parser, the private `encode_key_table`,
//! and the `__digstore_data`/`__digstore_data_end` extern-symbol scheme are all
//! removed.

use alloc::vec::Vec;
use digstore_core::datasection::{lookup_key as core_lookup_key, DataView};
use digstore_core::Bytes32;
use digstore_core::KeyTableEntry;

// Canonical section identifiers + key-table codec come straight from core, so
// the guest, compiler, and client all agree byte-for-byte.
pub use digstore_core::datasection::{encode_key_table, SectionId};

/// Parse failure when reading a data-section blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionError;

/// Guest-side view over a parsed data-section blob (delegates to the canonical
/// `digstore_core::datasection::DataView`).
pub struct DataSection<'a> {
    view: DataView<'a>,
}

impl<'a> DataSection<'a> {
    /// Parse a data-section blob slice (native-test path takes a blob directly;
    /// the wasm path passes the slice read from [`DIGS_DATA_OFFSET`]).
    ///
    /// [`DIGS_DATA_OFFSET`]: digstore_core::datasection::DIGS_DATA_OFFSET
    pub fn parse(raw: &'a [u8]) -> Result<Self, SectionError> {
        let view = DataView::parse(raw).map_err(|_| SectionError)?;
        Ok(DataSection { view })
    }

    /// Return the exact body slice for `id`, or `None` if absent.
    pub fn section(&self, id: SectionId) -> Option<&'a [u8]> {
        self.view.section(id)
    }

    /// Self-describing total blob length (`max(offset + len)` over all rows).
    pub fn total_len(&self) -> usize {
        self.view.total_len()
    }

    /// Store id (32 bytes, zero-padded if the section is short/absent).
    pub fn store_id(&self) -> Bytes32 {
        bytes32_or_zero(self.section(SectionId::StoreId))
    }

    /// Current generation root (32 bytes, zero-padded if short/absent).
    pub fn current_root(&self) -> Bytes32 {
        bytes32_or_zero(self.section(SectionId::CurrentRoot))
    }

    /// Linear scan of the KeyTable for a matching `static_key` (= retrieval key),
    /// using the canonical core codec. Callers gate misses into the decoy path.
    pub fn lookup_key(&self, retrieval_key: &Bytes32) -> Option<KeyTableEntry> {
        let body = self.section(SectionId::KeyTable)?;
        core_lookup_key(body, retrieval_key)
    }
}

fn bytes32_or_zero(section: Option<&[u8]>) -> Bytes32 {
    let s = section.unwrap_or(&[]);
    let mut a = [0u8; 32];
    let n = s.len().min(32);
    a[..n].copy_from_slice(&s[..n]);
    Bytes32(a)
}

/// Read the compiler-injected blob from the fixed linear-memory pointer
/// [`DIGS_DATA_OFFSET`] (contract D2): parse the `DIGS` header to learn
/// `total_len`, then build a [`DataSection`] over exactly that many bytes.
///
/// [`DIGS_DATA_OFFSET`]: digstore_core::datasection::DIGS_DATA_OFFSET
#[cfg(target_arch = "wasm32")]
pub fn embedded<'a>() -> DataSection<'a> {
    use digstore_core::datasection::DIGS_DATA_OFFSET;

    // Fixed header: magic(4) + version(1) + count(u32 BE).
    const HEADER_LEN: usize = 9;
    const ROW_LEN: usize = 10;

    let empty = || DataSection {
        view: DataView::parse(EMPTY_BLOB).unwrap(),
    };

    unsafe {
        let base = DIGS_DATA_OFFSET as *const u8;

        // 1) Read only the fixed header to learn the row count, so we never read
        //    past the injected blob (the compiler sizes memory to exactly cover
        //    DIGS_DATA_OFFSET + total_len, D2).
        let header = core::slice::from_raw_parts(base, HEADER_LEN);
        if &header[0..4] != b"DIGS" || header[4] != 1 {
            return empty();
        }
        let count = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;

        // 2) Read header + offset table only. We compute total_len = max(offset+len)
        //    directly from the rows here (we cannot call DataView::parse yet — the
        //    bodies are not in this slice, so its in-bounds check would reject the
        //    body offsets).
        let table_len = match count
            .checked_mul(ROW_LEN)
            .and_then(|t| t.checked_add(HEADER_LEN))
        {
            Some(n) => n,
            None => return empty(),
        };
        let head_and_rows = core::slice::from_raw_parts(base, table_len);
        let mut total_len = table_len;
        for i in 0..count {
            let p = HEADER_LEN + i * ROW_LEN;
            // row = id(u16) | offset(u32) | len(u32)
            let offset = u32::from_be_bytes([
                head_and_rows[p + 2],
                head_and_rows[p + 3],
                head_and_rows[p + 4],
                head_and_rows[p + 5],
            ]) as usize;
            let len = u32::from_be_bytes([
                head_and_rows[p + 6],
                head_and_rows[p + 7],
                head_and_rows[p + 8],
                head_and_rows[p + 9],
            ]) as usize;
            let end = match offset.checked_add(len) {
                Some(e) => e,
                None => return empty(),
            };
            if end > total_len {
                total_len = end;
            }
        }

        // 3) Read exactly total_len bytes and parse the full blob through core.
        let full = core::slice::from_raw_parts(base, total_len);
        DataSection::parse(full).unwrap_or_else(|_| empty())
    }
}

/// A valid empty blob: magic `DIGS`, version 1, zero offset rows. Used as the
/// fallback when no valid blob has been injected.
#[cfg(target_arch = "wasm32")]
const EMPTY_BLOB: &[u8] = b"DIGS\x01\x00\x00\x00\x00";

/// Build a [`DataSection`] from a caller-provided blob slice (native-test path).
/// Equivalent to [`DataSection::parse`]; named for symmetry with `embedded`.
pub fn from_blob(blob: &[u8]) -> Result<DataSection<'_>, SectionError> {
    DataSection::parse(blob)
}

/// Encode a list of `(id, body)` sections into a data-section blob using the
/// canonical core encoder. Convenience for guest tests that assemble blobs.
pub fn encode_blob(sections: &[(u16, Vec<u8>)]) -> Vec<u8> {
    digstore_core::datasection::encode_blob(sections)
}
