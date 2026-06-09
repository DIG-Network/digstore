//! Chia AugScheme BLS (CONVENTIONS C1).
//!
//! Public key material lives in this module as [`SecretKey`], [`PublicKey`], and
//! [`Signature`] (G1 = 48 bytes, G2 = 96 bytes). `digstore-host` and
//! `digstore-prover` consume these types (with the [`BlsSecretKey`] /
//! [`BlsPublicKey`] aliases). The guest does NOT use this module — it verifies
//! with pure-Rust `bls12_381`; the cross-impl parity fixtures (C8) prove the two
//! agree.
//!
//! Signing uses the Chia AugScheme: the public key is prepended to the message
//! and hashed with the Chia DST before signing into G2.

use crate::error::{BlsError, CryptoError};
use chia_bls::{
    sign as aug_sign, verify as aug_verify, PublicKey as ChiaPublicKey, SecretKey as ChiaSecretKey,
    Signature as ChiaSignature,
};
use digstore_core::{AttestationChallenge, Bytes32, Bytes48, Bytes96};

/// Host-side BLS signing key (wraps `chia_bls::SecretKey`, blst backend).
pub struct SecretKey(ChiaSecretKey);

/// BLS public key: a 48-byte G1 point.
#[derive(Clone)]
pub struct PublicKey(ChiaPublicKey);

/// BLS signature: a 96-byte G2 point.
#[derive(Clone)]
pub struct Signature(ChiaSignature);

/// Alias used by `digstore-host` / `digstore-prover` (CONVENTIONS C1).
pub type BlsSecretKey = SecretKey;
/// Alias used by `digstore-host` / `digstore-prover` (CONVENTIONS C1).
pub type BlsPublicKey = PublicKey;

impl SecretKey {
    /// Deterministically derive a signing key from a seed (Chia keygen).
    pub fn from_seed(seed: &[u8]) -> Self {
        SecretKey(ChiaSecretKey::from_seed(seed))
    }

    /// The 48-byte G1 public key for this signing key.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.public_key())
    }

    /// Sign `msg` under the Chia AugScheme (public key prepended, Chia DST).
    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(aug_sign(&self.0, msg))
    }
}

impl PublicKey {
    /// Serialize to the canonical 48-byte G1 encoding.
    pub fn to_bytes(&self) -> Bytes48 {
        Bytes48(self.0.to_bytes())
    }

    /// Parse a canonical 48-byte G1 public key. Returns
    /// `CryptoError::Bls(BlsError::InvalidPublicKey)` for non-canonical bytes.
    pub fn from_bytes(b: &Bytes48) -> Result<Self, CryptoError> {
        ChiaPublicKey::from_bytes(&b.0)
            .map(PublicKey)
            .map_err(|_| CryptoError::Bls(BlsError::InvalidPublicKey))
    }

    /// Verify a 96-byte AugScheme signature against this key and `msg`.
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        aug_verify(&sig.0, &self.0, msg)
    }
}

impl Signature {
    /// Serialize to the canonical 96-byte G2 encoding.
    pub fn to_bytes(&self) -> Bytes96 {
        Bytes96(self.0.to_bytes())
    }

    /// Parse a canonical 96-byte G2 signature. Returns
    /// `CryptoError::Bls(BlsError::InvalidSignature)` for non-canonical bytes.
    pub fn from_bytes(b: &Bytes96) -> Result<Self, CryptoError> {
        ChiaSignature::from_bytes(&b.0)
            .map(Signature)
            .map_err(|_| CryptoError::Bls(BlsError::InvalidSignature))
    }
}

// --- Free-function convenience helpers (byte-oriented) ---------------------

/// Deterministically derive a keypair from a 32-byte seed; returns the signing
/// key and its 48-byte G1 public key bytes.
pub fn bls_keygen(seed: &[u8; 32]) -> (SecretKey, Bytes48) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key().to_bytes();
    (sk, pk)
}

/// Sign `msg` under the Chia AugScheme; returns the 96-byte G2 signature bytes.
pub fn bls_sign(sk: &SecretKey, msg: &[u8]) -> Bytes96 {
    sk.sign(msg).to_bytes()
}

/// Verify a 96-byte AugScheme signature against 48-byte public-key bytes.
/// Returns `false` on any malformed input or invalid signature.
pub fn bls_verify(pk: &Bytes48, msg: &[u8], sig: &Bytes96) -> bool {
    let pk = match PublicKey::from_bytes(pk) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let sig = match Signature::from_bytes(sig) {
        Ok(s) => s,
        Err(_) => return false,
    };
    pk.verify(msg, &sig)
}

/// Canonical compressed encoding of the G1 identity (point at infinity):
/// compression+infinity flag bits set in the first byte, all coordinate bytes
/// zero (zcash/chia BLS12-381 serialization).
const G1_INFINITY: [u8; 48] = {
    let mut a = [0u8; 48];
    a[0] = 0xc0;
    a
};

/// Validate that `pk` is a canonical, NON-identity G1 public key, surfacing a
/// typed error. The identity (point at infinity) is rejected: it is never a
/// legitimate key and accepting it enables degenerate-signature / rogue-key
/// style abuses where a signature can verify under a "key" no one controls.
pub fn validate_public_key(pk: &Bytes48) -> Result<(), BlsError> {
    if pk.0 == G1_INFINITY {
        return Err(BlsError::InvalidPublicKey);
    }
    match PublicKey::from_bytes(pk) {
        Ok(_) => Ok(()),
        Err(_) => Err(BlsError::InvalidPublicKey),
    }
}

// --- Canonical signing-message builders ------------------------------------

/// Canonical push-authorization signing message (CONVENTIONS C7, paper §21.6):
/// `SHA-256(root || store_id)` (32 bytes). Argument order is `(root, store_id)`.
/// `digstore-remote` and `digstore-cli` delegate to this single source of truth.
pub fn push_signing_message(root: &Bytes32, store_id: &Bytes32) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&root.0);
    buf[32..].copy_from_slice(&store_id.0);
    crate::hash::sha256(&buf).0
}

/// Canonical node execution-proof signing message (paper §13.7, §16).
///
/// Binds the attestation-relevant fields of `ExecutionProof`:
///   `program_hash(32) || public_output(32) || chia_header_hash(32)
///    || height_be(4) || public_input(var)`.
/// `height` is encoded big-endian (Chia-compat rule).
pub fn node_signing_message(
    program_hash: &Bytes32,
    public_output: &Bytes32,
    chia_header_hash: &Bytes32,
    height: u32,
    public_input: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(100 + public_input.len());
    msg.extend_from_slice(&program_hash.0);
    msg.extend_from_slice(&public_output.0);
    msg.extend_from_slice(&chia_header_hash.0);
    msg.extend_from_slice(&height.to_be_bytes());
    msg.extend_from_slice(public_input);
    msg
}

/// Canonical attestation signing message (paper §12):
/// `nonce(32) || store_id(32) || timestamp_be(8)` (72 bytes).
pub fn attestation_signing_message(
    nonce: &[u8; 32],
    store_id: &[u8; 32],
    timestamp: u64,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(72);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(store_id);
    msg.extend_from_slice(&timestamp.to_be_bytes());
    msg
}

// --- High-level signing/verification over canonical messages ---------------

/// Push-authorization signature (CONVENTIONS C7, paper §21.6): sign
/// `SHA-256(root || store_id)` with the store's BLS key.
pub fn sign_push(sk: &SecretKey, root: &Bytes32, store_id: &Bytes32) -> Bytes96 {
    bls_sign(sk, &push_signing_message(root, store_id))
}

/// Verify a push-authorization signature against the store public key, using the
/// byte-identical canonical message (CONVENTIONS C7). `digstore-remote` calls
/// THIS to authorize a push.
pub fn verify_push(pk: &PublicKey, root: &Bytes32, store_id: &Bytes32, sig: &Bytes96) -> bool {
    let sig = match Signature::from_bytes(sig) {
        Ok(s) => s,
        Err(_) => return false,
    };
    pk.verify(&push_signing_message(root, store_id), &sig)
}

/// Node execution-proof signature (paper §13.7, §16). Signs the canonical
/// [`node_signing_message`].
#[allow(clippy::too_many_arguments)]
pub fn sign_node(
    node_sk: &SecretKey,
    program_hash: &Bytes32,
    public_output: &Bytes32,
    chia_header_hash: &Bytes32,
    height: u32,
    public_input: &[u8],
) -> Bytes96 {
    let msg =
        node_signing_message(program_hash, public_output, chia_header_hash, height, public_input);
    bls_sign(node_sk, &msg)
}

/// Host attestation signature (paper §12). Signs the canonical
/// [`attestation_signing_message`] over `nonce || store_id || timestamp_be`.
pub fn sign_attestation(host_sk: &SecretKey, challenge: &AttestationChallenge) -> Bytes96 {
    let msg =
        attestation_signing_message(&challenge.nonce, &challenge.store_id, challenge.timestamp);
    bls_sign(host_sk, &msg)
}
