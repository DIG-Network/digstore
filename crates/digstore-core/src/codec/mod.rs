//! Chia-streamable codec: BIG-ENDIAN fixed-width framing.
//!
//! DOCUMENTED DEVIATION 1: This codec is BIG-ENDIAN (Chia STREAMABLE),
//! NOT the paper's little-endian note. Chia compatibility wins.
//!
//! Framing rules (Chia STREAMABLE):
//! - `uintN`/`intN`: fixed-width big-endian.
//! - `Option<T>`: 1 tag byte (0=None, 1=Some) then `T`.
//! - `Vec<T>`: 4-byte BE count, then each item.
//! - `String`: 4-byte BE byte-length, then UTF-8 bytes.
//! - `Bytes32/48/96`: raw bytes, no length prefix.

pub mod primitives;
pub mod section;

use alloc::string::String;
use alloc::vec::Vec;

/// Error produced while decoding a byte stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Reached end of input before the value was fully read.
    UnexpectedEof,
    /// A tag/discriminant byte was out of range.
    InvalidTag(u8),
    /// UTF-8 validation failed for a String.
    InvalidUtf8,
    /// A magic / version / structural mismatch.
    Invalid(&'static str),
}

/// Append-only big-endian writer.
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder { buf: Vec::new() }
    }
    /// Write raw bytes with no length prefix.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
    /// Consume the encoder and return the accumulated bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
    /// Current length of the buffer (used by the section offset table).
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// Forward-only big-endian reader over a borrowed slice.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Decoder { data, pos: 0 }
    }
    /// Read exactly `n` raw bytes.
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Bytes remaining.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// Serialize `self` into an `Encoder` using big-endian Chia framing.
pub trait Encode {
    fn encode(&self, enc: &mut Encoder);
    /// Convenience: encode into a fresh `Vec<u8>`.
    fn to_bytes(&self) -> Vec<u8> {
        let mut enc = Encoder::new();
        self.encode(&mut enc);
        enc.finish()
    }
}

/// Deserialize `Self` from a `Decoder`.
pub trait Decode: Sized {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError>;
    /// Convenience: decode from a complete byte slice (does not require full consumption).
    fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(bytes);
        Self::decode(&mut dec)
    }
}

/// Helper used by `String`/manifest decode to validate UTF-8 in no_std.
pub(crate) fn utf8_from(bytes: &[u8]) -> Result<String, DecodeError> {
    core::str::from_utf8(bytes)
        .map(|s| s.into())
        .map_err(|_| DecodeError::InvalidUtf8)
}
