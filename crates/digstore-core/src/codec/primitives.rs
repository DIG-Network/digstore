//! Big-endian Chia framing for primitive types.

use super::{utf8_from, Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::string::String;
use alloc::vec::Vec;

macro_rules! impl_uint {
    ($t:ty, $n:expr) => {
        impl Encode for $t {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(&self.to_be_bytes());
            }
        }
        impl Decode for $t {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok(<$t>::from_be_bytes(arr))
            }
        }
    };
}

impl_uint!(u8, 1);
impl_uint!(u16, 2);
impl_uint!(u32, 4);
impl_uint!(u64, 8);

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, enc: &mut Encoder) {
        match self {
            None => enc.write_bytes(&[0u8]),
            Some(v) => {
                enc.write_bytes(&[1u8]);
                v.encode(enc);
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let tag = dec.read_bytes(1)?[0];
        match tag {
            0 => Ok(None),
            1 => Ok(Some(T::decode(dec)?)),
            other => Err(DecodeError::InvalidTag(other)),
        }
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, enc: &mut Encoder) {
        (self.len() as u32).encode(enc);
        for item in self {
            item.encode(enc);
        }
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let count = u32::decode(dec)? as usize;
        let mut out = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            out.push(T::decode(dec)?);
        }
        Ok(out)
    }
}

impl Encode for String {
    fn encode(&self, enc: &mut Encoder) {
        let bytes = self.as_bytes();
        (bytes.len() as u32).encode(enc);
        enc.write_bytes(bytes);
    }
}

impl Decode for String {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let len = u32::decode(dec)? as usize;
        let bytes = dec.read_bytes(len)?;
        utf8_from(bytes)
    }
}

macro_rules! impl_fixed_array {
    ($n:expr) => {
        impl Encode for [u8; $n] {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(self);
            }
        }
        impl Decode for [u8; $n] {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok(arr)
            }
        }
    };
}

impl_fixed_array!(4); // used by tests + small fixed fields
impl_fixed_array!(32);
impl_fixed_array!(48);
impl_fixed_array!(96);
