//! Fixed-width byte newtypes with hex + codec + optional serde.

use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::error::CoreError;
use alloc::format;
use alloc::string::String;

macro_rules! bytes_newtype {
    ($name:ident, $n:expr) => {
        /// Fixed-width byte container (raw bytes on the wire, no length prefix).
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub [u8; $n]);

        impl $name {
            pub const LEN: usize = $n;

            /// Lowercase hex (no `0x` prefix).
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }

            /// Parse from lowercase/uppercase hex; must be exactly `2*LEN` chars.
            pub fn from_hex(s: &str) -> Result<Self, CoreError> {
                let bytes = hex::decode(s)
                    .map_err(|e| CoreError::Parse(format!("hex: {e}")))?;
                if bytes.len() != $n {
                    return Err(CoreError::Parse(format!(
                        "expected {} bytes, got {}",
                        $n,
                        bytes.len()
                    )));
                }
                let mut arr = [0u8; $n];
                arr.copy_from_slice(&bytes);
                Ok($name(arr))
            }

            pub fn as_bytes(&self) -> &[u8; $n] {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name([0u8; $n])
            }
        }

        impl Encode for $name {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(&self.0);
            }
        }

        impl Decode for $name {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok($name(arr))
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_hex())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::from_hex(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

bytes_newtype!(Bytes32, 32);
bytes_newtype!(Bytes48, 48);
bytes_newtype!(Bytes96, 96);
