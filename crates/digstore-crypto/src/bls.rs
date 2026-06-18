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
use digstore_core::{AttestationChallenge, Bytes32, Bytes48, Bytes96, Tombstone};

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

// --- Per-role BLS domain-separation tags (SECURITY.md residual #2) ----------
//
// Each canonical signing-message builder PREPENDS a DISTINCT role tag before the
// payload, so a signature produced for one role (push / node-proof / attestation)
// can never be replayed as a signature for another role, even if the underlying
// payload bytes happened to coincide. This is defense-in-depth ON TOP of the Chia
// AugScheme (which already prepends the public key + the Chia DST): the AugScheme
// binds the signer's key, while these tags bind the protocol role. Both the sign
// and the verify paths go through these builders, so adding the tag here covers
// both directions.

/// Role tag for push-authorization signatures (`sign_push` / `verify_push`).
pub const PUSH_DST: &[u8] = b"digstore:push:v1";
/// Role tag for node execution-proof signatures (`sign_node`).
///
/// NOTE: these bytes (`b"digstore:node:v1"`) intentionally COINCIDE with
/// `digstore_core::merkle::NODE_TAG`. This is safe — it is not a cross-domain
/// collision — because the two tags live in disjoint preimage shapes that are
/// never compared: `NODE_TAG` prefixes a fixed-length `SHA-256(tag || left(32)
/// || right(32))` Merkle-node fold (80-byte preimage), whereas `NODE_DST`
/// prefixes a BLS AugScheme signing message of `tag || program_hash(32) ||
/// public_output(32) || chia_header_hash(32) || height(4) || public_input(var)`
/// (≥116 bytes, and AugScheme additionally binds the signer's public key). No
/// code path ever verifies a Merkle fold as a BLS message or vice-versa, so the
/// shared bytes cannot enable a replay. The DISTINCT-tag guarantee in the block
/// comment above is about the BLS *roles* (push / node / attestation / tomb /
/// req), which DO use mutually distinct tags; it makes no claim against the
/// Merkle domain.
pub const NODE_DST: &[u8] = b"digstore:node:v1";
/// Role tag for root-revocation tombstone signatures (`sign_tombstone` /
/// `verify_tombstone`, SECURITY.md residual #1 Layer 1). Distinct from the
/// push/node/attestation tags so a tombstone signature can never be replayed as
/// (nor forged from) a push, node-proof, or attestation signature, even if the
/// underlying payload bytes happened to coincide.
pub const TOMB_DST: &[u8] = b"digstore:tomb:v1";
/// Role tag for per-request remote-protocol authentication signatures
/// (`sign_request` / `verify_request`, paper §21.9). Every dig:// remote request
/// (fetch / roots / module / content / proof / push / tombstone) is signed by the
/// CLI's IDENTITY key over [`request_signing_message`]; the role tag keeps such a
/// request signature from ever being replayable as a push / node-proof /
/// attestation / tombstone signature, and vice-versa.
pub const REQ_DST: &[u8] = b"digstore:req:v1";
/// Role tag for host attestation signatures (`sign_attestation`). Re-exported
/// from `digstore_core` so the producer here and the guest's `build_challenge`
/// verifier share ONE definition and stay byte-identical.
pub use digstore_core::ATTEST_DST;

// --- Canonical signing-message builders ------------------------------------

/// Canonical push-authorization signing message (CONVENTIONS C7, paper §21.6):
/// `SHA-256(PUSH_DST || root || store_id)` (32 bytes). Argument order is
/// `(root, store_id)`. `digstore-remote` and `digstore-cli` delegate to this
/// single source of truth. The role tag is folded into the hashed preimage so
/// the signed message stays a fixed 32 bytes.
pub fn push_signing_message(root: &Bytes32, store_id: &Bytes32) -> [u8; 32] {
    let mut buf = Vec::with_capacity(PUSH_DST.len() + 64);
    buf.extend_from_slice(PUSH_DST);
    buf.extend_from_slice(&root.0);
    buf.extend_from_slice(&store_id.0);
    crate::sha256(&buf).0
}

/// Canonical node execution-proof signing message (paper §13.7, §16).
///
/// Binds the attestation-relevant fields of `ExecutionProof`, prefixed by the
/// node role tag:
///   `NODE_DST || program_hash(32) || public_output(32) || chia_header_hash(32)
///    || height_be(4) || public_input(var)`.
/// `height` is encoded big-endian (Chia-compat rule).
pub fn node_signing_message(
    program_hash: &Bytes32,
    public_output: &Bytes32,
    chia_header_hash: &Bytes32,
    height: u32,
    public_input: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(NODE_DST.len() + 100 + public_input.len());
    msg.extend_from_slice(NODE_DST);
    msg.extend_from_slice(&program_hash.0);
    msg.extend_from_slice(&public_output.0);
    msg.extend_from_slice(&chia_header_hash.0);
    msg.extend_from_slice(&height.to_be_bytes());
    msg.extend_from_slice(public_input);
    msg
}

/// Canonical attestation signing message (paper §12):
/// `ATTEST_DST || nonce(32) || store_id(32) || timestamp_be(8)`.
///
/// MUST stay byte-identical to the guest's `build_challenge` (which prepends the
/// same `ATTEST_DST`), because at runtime the host signs the exact challenge
/// bytes the guest builds and the guest verifies that same buffer.
pub fn attestation_signing_message(
    nonce: &[u8; 32],
    store_id: &[u8; 32],
    timestamp: u64,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ATTEST_DST.len() + 72);
    msg.extend_from_slice(ATTEST_DST);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(store_id);
    msg.extend_from_slice(&timestamp.to_be_bytes());
    msg
}

/// Canonical tombstone signing message (SECURITY.md residual #1 Layer 1):
/// `SHA-256(TOMB_DST || canonical(Tombstone))` (32 bytes). The role tag is folded
/// into the hashed preimage so the signed message stays a fixed 32 bytes, exactly
/// like [`push_signing_message`]. Both `sign_tombstone` and `verify_tombstone` go
/// through this single builder so producer and verifier stay byte-identical.
pub fn tombstone_signing_message(t: &Tombstone) -> [u8; 32] {
    let canonical = t.canonical();
    let mut buf = Vec::with_capacity(TOMB_DST.len() + canonical.len());
    buf.extend_from_slice(TOMB_DST);
    buf.extend_from_slice(&canonical);
    crate::sha256(&buf).0
}

/// Canonical per-request authentication signing message (paper §21.9):
/// `SHA-256(REQ_DST || len(method) || method || store_id(32) || timestamp_be(8) || nonce(32))`
/// (32 bytes). `method` is the logical operation (`"fetch"`, `"roots"`, `"module"`,
/// `"content"`, `"proof"`, `"push"`, `"tombstone"`) — binding it stops a read-auth
/// signature from being replayed as a write. `timestamp` (unix seconds) lets the
/// server reject stale/replayed requests within a freshness window, and `nonce`
/// (32 random bytes) makes each signed request unique. The method is length-prefixed
/// (big-endian u32) so the preimage is unambiguous. Both the CLI signer and the
/// server verifier go through this single builder so they stay byte-identical.
pub fn request_signing_message(
    method: &str,
    store_id: &Bytes32,
    timestamp: u64,
    nonce: &[u8; 32],
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(REQ_DST.len() + 4 + method.len() + 32 + 8 + 32);
    buf.extend_from_slice(REQ_DST);
    buf.extend_from_slice(&(method.len() as u32).to_be_bytes());
    buf.extend_from_slice(method.as_bytes());
    buf.extend_from_slice(&store_id.0);
    buf.extend_from_slice(&timestamp.to_be_bytes());
    buf.extend_from_slice(nonce);
    crate::sha256(&buf).0
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

/// Per-request authentication signature (paper §21.9): sign the canonical
/// [`request_signing_message`] with the CLI's identity key. Attached to every
/// dig:// remote request so the server can authenticate the caller.
pub fn sign_request(
    sk: &SecretKey,
    method: &str,
    store_id: &Bytes32,
    timestamp: u64,
    nonce: &[u8; 32],
) -> Bytes96 {
    bls_sign(
        sk,
        &request_signing_message(method, store_id, timestamp, nonce),
    )
}

/// Verify a per-request authentication signature against the caller's identity
/// public key, using the byte-identical canonical message (paper §21.9). Returns
/// false on a malformed signature. Freshness (timestamp window) and any
/// authorization (e.g. the identity is the store owner for a push) are enforced
/// by the caller; this proves only that `pk` signed exactly this request.
pub fn verify_request(
    pk: &PublicKey,
    method: &str,
    store_id: &Bytes32,
    timestamp: u64,
    nonce: &[u8; 32],
    sig: &Bytes96,
) -> bool {
    let sig = match Signature::from_bytes(sig) {
        Ok(s) => s,
        Err(_) => return false,
    };
    pk.verify(
        &request_signing_message(method, store_id, timestamp, nonce),
        &sig,
    )
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
    let msg = node_signing_message(
        program_hash,
        public_output,
        chia_header_hash,
        height,
        public_input,
    );
    bls_sign(node_sk, &msg)
}

/// Host attestation signature (paper §12). Signs the canonical
/// [`attestation_signing_message`] over `nonce || store_id || timestamp_be`.
pub fn sign_attestation(host_sk: &SecretKey, challenge: &AttestationChallenge) -> Bytes96 {
    let msg =
        attestation_signing_message(&challenge.nonce, &challenge.store_id, challenge.timestamp);
    bls_sign(host_sk, &msg)
}

/// Root-revocation tombstone signature (SECURITY.md residual #1 Layer 1). Signs
/// the canonical [`tombstone_signing_message`] over `TOMB_DST || canonical(t)`
/// with the store's BLS key. Mirrors [`sign_push`].
pub fn sign_tombstone(sk: &SecretKey, t: &Tombstone) -> Bytes96 {
    bls_sign(sk, &tombstone_signing_message(t))
}

/// Verify a tombstone signature against the store public key, using the
/// byte-identical canonical message. Mirrors [`verify_push`]: returns `false` on
/// any malformed signature or verification failure (fail-closed at the caller).
pub fn verify_tombstone(pk: &PublicKey, t: &Tombstone, sig: &Bytes96) -> bool {
    let sig = match Signature::from_bytes(sig) {
        Ok(s) => s,
        Err(_) => return false,
    };
    pk.verify(&tombstone_signing_message(t), &sig)
}

#[cfg(test)]
mod request_auth_tests {
    use super::*;

    fn keypair(seed: u8) -> (SecretKey, PublicKey) {
        let (sk, pk_bytes) = bls_keygen(&[seed; 32]);
        (sk, PublicKey::from_bytes(&pk_bytes).unwrap())
    }

    #[test]
    fn request_signature_round_trips() {
        let (sk, pk) = keypair(7);
        let store = Bytes32([9u8; 32]);
        let nonce = [3u8; 32];
        let sig = sign_request(&sk, "module", &store, 1_700_000_000, &nonce);
        assert!(verify_request(
            &pk,
            "module",
            &store,
            1_700_000_000,
            &nonce,
            &sig
        ));
    }

    #[test]
    fn request_signature_is_bound_to_method_store_time_and_nonce() {
        let (sk, pk) = keypair(7);
        let store = Bytes32([9u8; 32]);
        let nonce = [3u8; 32];
        let ts = 1_700_000_000;
        let sig = sign_request(&sk, "module", &store, ts, &nonce);
        // A signature over "module" must NOT verify as a "push" (no cross-method replay).
        assert!(!verify_request(&pk, "push", &store, ts, &nonce, &sig));
        // Different store / timestamp / nonce all break verification.
        assert!(!verify_request(
            &pk,
            "module",
            &Bytes32([8u8; 32]),
            ts,
            &nonce,
            &sig
        ));
        assert!(!verify_request(&pk, "module", &store, ts + 1, &nonce, &sig));
        assert!(!verify_request(&pk, "module", &store, ts, &[4u8; 32], &sig));
    }

    #[test]
    fn request_message_differs_from_push_message_for_same_bytes() {
        // Defense-in-depth: the REQ_DST tag keeps a request message distinct from a
        // push message even if the payload bytes coincide.
        let store = Bytes32([9u8; 32]);
        let root = Bytes32([9u8; 32]);
        let nonce = [0u8; 32];
        assert_ne!(
            request_signing_message("push", &store, 0, &nonce),
            push_signing_message(&root, &store)
        );
    }
}
