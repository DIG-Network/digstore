//! Key-table entry and path-walk cursor (paper 8.4).

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::vec::Vec;

/// A key-table entry mapping a resource's static key + generation to its chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyTableEntry {
    pub static_key: Bytes32,
    pub generation: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub total_size: u64,
}

impl Encode for KeyTableEntry {
    fn encode(&self, enc: &mut Encoder) {
        self.static_key.encode(enc);
        self.generation.encode(enc);
        self.chunk_indices.encode(enc);
        self.total_size.encode(enc);
    }
}

impl Decode for KeyTableEntry {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(KeyTableEntry {
            static_key: Bytes32::decode(dec)?,
            generation: Bytes32::decode(dec)?,
            chunk_indices: Vec::<u32>::decode(dec)?,
            total_size: u64::decode(dec)?,
        })
    }
}

/// A walk over a resource's chunk indices with a resumable cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathWalk {
    pub resource_key: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub cursor: usize,
}

impl Encode for PathWalk {
    fn encode(&self, enc: &mut Encoder) {
        self.resource_key.encode(enc);
        self.chunk_indices.encode(enc);
        (self.cursor as u64).encode(enc);
    }
}

impl Decode for PathWalk {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let resource_key = Bytes32::decode(dec)?;
        let chunk_indices = Vec::<u32>::decode(dec)?;
        let cursor = u64::decode(dec)? as usize;
        Ok(PathWalk {
            resource_key,
            chunk_indices,
            cursor,
        })
    }
}
