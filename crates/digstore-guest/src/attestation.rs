//! Attestation (§12). The guest issues a fresh challenge (nonce from
//! host_random_bytes, store_id, current time), the host signs it with its BLS
//! key (chia-bls/blst, AugScheme), and the guest verifies the returned G2
//! signature against an embedded trusted G1 key set using pure-Rust bls12_381.
//! Failure (untrusted key / bad sig / stale) -> content calls return decoys.
//!
//! Chia AugScheme: the message actually signed is `pubkey || message`, hashed to
//! G2 with the DST `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_AUG_`. Verification is
//! the pairing check `e(pk, H(pubkey||message)) == e(g1, sig)`.

use alloc::vec::Vec;
use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G2Affine, G2Projective};
// digest-0.9-compatible SHA-256 for the bls12_381 expand-message step (see Cargo.toml).
use sha2_v09::Sha256;

const FRESHNESS_SECS: u64 = 300; // attestation valid for 5 minutes

/// Chia AugScheme hash-to-curve DST for G2 signatures.
const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_AUG_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationError {
    UntrustedKey,
    BadSignature,
    Stale,
    Malformed,
}

pub struct TrustedSet {
    keys: Vec<[u8; 48]>,
}

impl TrustedSet {
    pub fn from_pubkeys(keys: &[[u8; 48]]) -> Self {
        TrustedSet { keys: keys.to_vec() }
    }
    pub fn contains(&self, pk: &[u8; 48]) -> bool {
        self.keys.iter().any(|k| k == pk)
    }
}

/// Serialize the AttestationChallenge: nonce(32) || store_id(32) || time(u64 BE).
pub fn build_challenge(nonce: [u8; 32], store_id: [u8; 32], time: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(72);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&store_id);
    out.extend_from_slice(&time.to_be_bytes());
    out
}

/// Chia AugScheme: the message that is actually signed is `pubkey || message`.
fn aug_message(pubkey: &[u8; 48], message: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(48 + message.len());
    m.extend_from_slice(pubkey);
    m.extend_from_slice(message);
    m
}

/// Hash an augmented message to G2 using the Chia AUG DST.
fn hash_to_g2(msg: &[u8]) -> G2Affine {
    let p = <G2Projective as HashToCurve<ExpandMsgXmd<Sha256>>>::hash_to_curve(msg, DST);
    G2Affine::from(p)
}

/// Pure-Rust bls12_381 verification of a Chia AugScheme signature.
/// Returns `false` on any malformed point or a failed pairing.
pub fn bls_aug_verify(pubkey: &[u8; 48], message: &[u8], signature: &[u8; 96]) -> bool {
    let pk = match Option::<G1Affine>::from(G1Affine::from_compressed(pubkey)) {
        Some(p) => p,
        None => return false,
    };
    let sig = match Option::<G2Affine>::from(G2Affine::from_compressed(signature)) {
        Some(s) => s,
        None => return false,
    };
    let aug = aug_message(pubkey, message);
    let h = hash_to_g2(&aug);
    // e(pk, H(aug)) == e(g1, sig)
    let lhs = bls12_381::pairing(&pk, &h);
    let rhs = bls12_381::pairing(&G1Affine::generator(), &sig);
    lhs == rhs
}

/// Verify a host attestation: trusted-key membership, AugScheme BLS verify, freshness.
pub fn verify_attestation(
    trusted: &TrustedSet,
    message: &[u8],
    pubkey: &[u8; 48],
    signature: &[u8; 96],
    signed_time: u64,
    now: u64,
) -> Result<(), AttestationError> {
    if !trusted.contains(pubkey) {
        return Err(AttestationError::UntrustedKey);
    }
    if now.saturating_sub(signed_time) > FRESHNESS_SECS || now < signed_time {
        return Err(AttestationError::Stale);
    }
    // Validate point encodings up front so a malformed point is distinguishable.
    if Option::<G1Affine>::from(G1Affine::from_compressed(pubkey)).is_none()
        || Option::<G2Affine>::from(G2Affine::from_compressed(signature)).is_none()
    {
        return Err(AttestationError::Malformed);
    }
    if bls_aug_verify(pubkey, message, signature) {
        Ok(())
    } else {
        Err(AttestationError::BadSignature)
    }
}
