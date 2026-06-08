//! Metadata exports (§6.2). Pure functions returning bytes for the data-returning
//! ABI exports. `get_metadata` returns the plaintext manifest and is explicitly
//! NOT gated (it is public discovery info); all others are also ungated reads of
//! the embedded data section.

use crate::datasection::{DataSection, SectionId};
use alloc::vec::Vec;
use digstore_core::{Bytes32, Bytes48};

pub fn store_id(ds: &DataSection) -> Bytes32 {
    ds.store_id()
}

pub fn current_roothash(ds: &DataSection) -> Bytes32 {
    ds.current_root()
}

/// Root history section bytes: u32 BE count then 32-byte roots (newest last).
pub fn roothash_history(ds: &DataSection) -> Vec<u8> {
    ds.section(SectionId::RootHistory).unwrap_or(&[]).to_vec()
}

pub fn public_key(ds: &DataSection) -> Bytes48 {
    let s = ds.section(SectionId::PublicKey).unwrap_or(&[]);
    let mut a = [0u8; 48];
    a[..s.len().min(48)].copy_from_slice(&s[..s.len().min(48)]);
    Bytes48(a)
}

/// Plaintext manifest JSON, returned verbatim and ungated.
pub fn metadata_bytes(ds: &DataSection) -> Vec<u8> {
    ds.section(SectionId::Metadata).unwrap_or(&[]).to_vec()
}

/// Authentication info section (issuer/jwks-uri/audience hints), ungated bytes.
pub fn authentication_info(ds: &DataSection) -> Vec<u8> {
    ds.section(SectionId::AuthInfo).unwrap_or(&[]).to_vec()
}
