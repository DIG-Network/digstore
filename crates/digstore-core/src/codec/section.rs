//! Data-section header: magic `DIGS`, u8 format_version=1, then an offset table.

use super::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::vec::Vec;

/// Magic bytes at the start of a Digstore data section.
pub const DIGS_MAGIC: &[u8; 4] = b"DIGS";
/// Current data-section format version.
pub const FORMAT_VERSION: u8 = 1;

/// One entry in the section offset table: a logical section id plus its
/// byte offset and length within the surrounding data blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionEntry {
    pub id: u32,
    pub offset: u64,
    pub length: u64,
}

impl Encode for SectionEntry {
    fn encode(&self, enc: &mut Encoder) {
        self.id.encode(enc);
        self.offset.encode(enc);
        self.length.encode(enc);
    }
}

impl Decode for SectionEntry {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(SectionEntry {
            id: u32::decode(dec)?,
            offset: u64::decode(dec)?,
            length: u64::decode(dec)?,
        })
    }
}

/// Data-section header: magic + version + offset table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionHeader {
    pub format_version: u8,
    pub entries: Vec<SectionEntry>,
}

impl SectionHeader {
    /// Look up `(offset, length)` for a section id.
    pub fn find(&self, id: u32) -> Option<(u64, u64)> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| (e.offset, e.length))
    }
}

impl Encode for SectionHeader {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(DIGS_MAGIC);
        self.format_version.encode(enc);
        self.entries.encode(enc); // Vec<SectionEntry>: 4-byte BE count then entries
    }
}

impl Decode for SectionHeader {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let magic = dec.read_bytes(4)?;
        if magic != DIGS_MAGIC {
            return Err(DecodeError::Invalid("bad DIGS magic"));
        }
        let format_version = u8::decode(dec)?;
        if format_version != FORMAT_VERSION {
            return Err(DecodeError::Invalid("unknown format version"));
        }
        let entries = Vec::<SectionEntry>::decode(dec)?;
        Ok(SectionHeader {
            format_version,
            entries,
        })
    }
}
