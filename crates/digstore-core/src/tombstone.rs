//! Signed root-revocation tombstones (Layer 1 of the key-rotation/revocation
//! design, SECURITY.md residual #1).
//!
//! A [`Tombstone`] lets a publisher retract a previously published root (or the
//! whole store) with a record signed by the store's BLS key. Remotes persist
//! tombstones per store and return the active set in the store descriptor;
//! clients verify each tombstone's signature against the store-id-bound module
//! key and refuse (fail-closed) to install or advance to a revoked root.
//!
//! This module owns only the TYPE and its canonical byte encoding (the same
//! big-endian Chia framing used by the rest of `digstore-core`). The signing
//! message (with its per-role domain-separation tag) and the BLS sign/verify
//! live in `digstore-crypto`, mirroring the push/node/attestation split.

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};

/// Why a root/store was revoked. The numeric value is part of the canonical
/// encoding (and therefore the signed bytes), so a tampered reason fails
/// verification. Unknown values decode as [`RevocationReason::Unspecified`] so a
/// newer publisher's reason code never makes an older client crash; the
/// signature still binds the exact byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RevocationReason {
    /// 0 — no reason given.
    Unspecified = 0,
    /// 1 — the signing key (or content) is believed compromised.
    Compromise = 1,
    /// 2 — superseded by a newer generation (informational).
    Superseded = 2,
    /// 3 — administrative/legal takedown.
    Takedown = 3,
}

impl RevocationReason {
    /// The on-wire byte for this reason.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Map a wire byte to a reason. Unknown bytes map to `Unspecified` (the
    /// canonical encoding still preserves the exact byte through `Tombstone`'s
    /// stored `reason` field, so this is only the human-facing label).
    pub fn from_u8(b: u8) -> Self {
        match b {
            1 => RevocationReason::Compromise,
            2 => RevocationReason::Superseded,
            3 => RevocationReason::Takedown,
            _ => RevocationReason::Unspecified,
        }
    }
}

/// What a tombstone retracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TombstoneScope {
    /// Retract a single generation root.
    Root(Bytes32),
    /// Retract the whole store (every root up to `not_after`).
    Store,
}

impl TombstoneScope {
    /// Discriminant byte used in the canonical encoding: 0 = Root, 1 = Store.
    fn tag(&self) -> u8 {
        match self {
            TombstoneScope::Root(_) => 0,
            TombstoneScope::Store => 1,
        }
    }
}

/// A signed root-revocation record (design §2 Layer 1).
///
/// The signature itself is NOT part of this struct — it is produced over the
/// canonical [`Tombstone::canonical`] bytes (with the per-role domain tag) by
/// `digstore_crypto::sign_tombstone` and carried alongside the record on the
/// wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tombstone {
    /// The store this tombstone applies to (binds the record to one identity).
    pub store_id: Bytes32,
    /// Which root (or the whole store) is retracted.
    pub scope: TombstoneScope,
    /// Unix seconds; the revocation's own timestamp.
    pub not_after: u64,
    /// Why it was revoked (see [`RevocationReason`]).
    pub reason: u8,
}

impl Tombstone {
    /// Convenience constructor for a single-root revocation.
    pub fn root(
        store_id: Bytes32,
        root: Bytes32,
        not_after: u64,
        reason: RevocationReason,
    ) -> Self {
        Tombstone {
            store_id,
            scope: TombstoneScope::Root(root),
            not_after,
            reason: reason.as_u8(),
        }
    }

    /// Convenience constructor for a whole-store revocation.
    pub fn store(store_id: Bytes32, not_after: u64, reason: RevocationReason) -> Self {
        Tombstone {
            store_id,
            scope: TombstoneScope::Store,
            not_after,
            reason: reason.as_u8(),
        }
    }

    /// The canonical, deterministic byte encoding of this tombstone (big-endian
    /// Chia framing). This is the exact preimage the signing message is built
    /// over, so it MUST be stable: `store_id(32) || scope_tag(1) ||
    /// [root(32) if Root] || not_after_be(8) || reason(1)`.
    pub fn canonical(&self) -> alloc::vec::Vec<u8> {
        self.to_bytes()
    }
}

impl Encode for Tombstone {
    fn encode(&self, enc: &mut Encoder) {
        self.store_id.encode(enc);
        enc.write_bytes(&[self.scope.tag()]);
        if let TombstoneScope::Root(root) = &self.scope {
            root.encode(enc);
        }
        self.not_after.encode(enc);
        enc.write_bytes(&[self.reason]);
    }
}

impl Decode for Tombstone {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let store_id = Bytes32::decode(dec)?;
        let scope_tag = dec.read_bytes(1)?[0];
        let scope = match scope_tag {
            0 => TombstoneScope::Root(Bytes32::decode(dec)?),
            1 => TombstoneScope::Store,
            other => return Err(DecodeError::InvalidTag(other)),
        };
        let not_after = u64::decode(dec)?;
        let reason = dec.read_bytes(1)?[0];
        Ok(Tombstone {
            store_id,
            scope,
            not_after,
            reason,
        })
    }
}
