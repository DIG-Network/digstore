#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod abi;
pub mod bytes;
pub mod codec;
pub mod config;
pub mod error;
pub mod hash;
pub mod keytable;
pub mod manifest;
pub mod merkle;
pub mod urn;
pub mod wire;

pub use error::{CoreError, ErrorCode};
pub use abi::{is_error, pack_ptr_len, unpack_ptr_len};
pub use codec::{Decode, DecodeError, Decoder, Encode, Encoder};
pub use bytes::{Bytes32, Bytes48, Bytes96};
pub use hash::sha256;

/// Alias module so `digstore_core::types::Bytes32` resolves (host/guest use this path).
pub mod types {
    pub use crate::bytes::{Bytes32, Bytes48, Bytes96};
}
