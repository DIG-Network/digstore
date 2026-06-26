//! The canonical `Capsule` identity — the single source of truth for
//! `(storeId, rootHash)` across the whole ecosystem.
//!
//! A **capsule** is one immutable store generation: the pair
//! `(store_id, root_hash)`, written canonically as `storeId:rootHash`
//! (lowercase hex `:` lowercase hex, matching [`Urn`](crate::urn::Urn)'s hex
//! convention via [`Bytes32::to_hex`]). A **store is a sequence of capsules** —
//! one per commit / root advance — identified by its `store_id`; each capsule is
//! a specific, on-chain-anchored root of that store. See the superproject
//! `SYSTEM.md` → "Core concept — the capsule".
//!
//! This is purely a *naming* layer over the existing `(store_id, root_hash)`
//! pair. It MUST NOT change any frozen wire format: the URN `canonical()` string
//! and `retrieval_key()` derivation are untouched — a capsule just gives that
//! pair a canonical name and a stable type to pass around.

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::error::CoreError;
use alloc::format;
use alloc::string::{String, ToString};

/// The identity of one immutable store generation: `(store_id, root_hash)`.
///
/// Canonical string form is `storeId:rootHash` (lowercase hex on both sides).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Capsule {
    pub store_id: Bytes32,
    pub root_hash: Bytes32,
}

impl Capsule {
    /// Render the canonical capsule string: `storeId:rootHash` (lowercase hex).
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.store_id.to_hex(), self.root_hash.to_hex())
    }

    /// Parse a canonical capsule string `storeId:rootHash`.
    ///
    /// Requires exactly two `:`-separated segments, each a valid 32-byte hex
    /// string. Missing/extra segments and bad hex are rejected.
    pub fn from_canonical(s: &str) -> Result<Capsule, CoreError> {
        let mut parts = s.split(':');
        let store_id_hex = parts
            .next()
            .ok_or_else(|| CoreError::Parse("capsule: missing store id".to_string()))?;
        let root_hash_hex = parts
            .next()
            .ok_or_else(|| CoreError::Parse("capsule: missing root hash".to_string()))?;
        if parts.next().is_some() {
            return Err(CoreError::Parse(
                "capsule: too many ':' segments".to_string(),
            ));
        }
        let store_id = Bytes32::from_hex(store_id_hex)?;
        let root_hash = Bytes32::from_hex(root_hash_hex)?;
        Ok(Capsule {
            store_id,
            root_hash,
        })
    }
}

impl core::fmt::Display for Capsule {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.store_id.to_hex(), self.root_hash.to_hex())
    }
}

impl Encode for Capsule {
    fn encode(&self, enc: &mut Encoder) {
        self.store_id.encode(enc);
        self.root_hash.encode(enc);
    }
}

impl Decode for Capsule {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Capsule {
            store_id: Bytes32::decode(dec)?,
            root_hash: Bytes32::decode(dec)?,
        })
    }
}
