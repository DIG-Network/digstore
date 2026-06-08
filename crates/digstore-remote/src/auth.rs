//! Push authorization (§21.6, CONVENTIONS C7).
//!
//! This module does NOT define its own push-signing message. It delegates to
//! `digstore_crypto::{push_signing_message, verify_push}`, the single source of
//! truth shared with `digstore-cli`. The canonical message is
//! `SHA-256(root || store_id)` and the argument order is `(root, store_id)`
//! everywhere.

use digstore_core::{Bytes32, Bytes48, Bytes96};

/// Re-export of the canonical push-signing message builder (CONVENTIONS C7).
/// Message = `SHA-256(root || store_id)`. Argument order is `(root, store_id)`.
pub use digstore_crypto::push_signing_message;

/// Parsed push-authorization inputs extracted from request headers/body.
#[derive(Debug, Clone)]
pub struct PushAuth {
    pub signature: Bytes96,
    pub bearer: Option<String>,
}

/// Verify the publisher BLS signature over the canonical push message (§21.6).
/// Delegates to `digstore_crypto::verify_push` (Chia AugScheme). Returns true on
/// a valid signature, false on any malformed input or verification failure.
///
/// Argument order is `(store_public_key, root, store_id, signature)`; internally
/// the crypto crate builds `SHA-256(root || store_id)` (CONVENTIONS C7).
pub fn verify_push_signature(
    store_public_key: &Bytes48,
    root: &Bytes32,
    store_id: &Bytes32,
    signature: &Bytes96,
) -> bool {
    let pk = match digstore_crypto::bls::PublicKey::from_bytes(store_public_key) {
        Ok(p) => p,
        Err(_) => return false,
    };
    digstore_crypto::verify_push(&pk, root, store_id, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(x: u8) -> Bytes32 {
        Bytes32([x; 32])
    }

    #[test]
    fn message_binds_store_id_and_root() {
        // canonical order (root, store_id)
        let m1 = push_signing_message(&b32(2), &b32(1));
        let m2 = push_signing_message(&b32(2), &b32(9)); // different store id
        let m3 = push_signing_message(&b32(3), &b32(1)); // different root
        assert_ne!(m1, m2);
        assert_ne!(m1, m3);
        // deterministic
        assert_eq!(m1, push_signing_message(&b32(2), &b32(1)));
    }

    #[test]
    fn valid_signature_verifies_and_tamper_fails() {
        // host-side keygen + sign (Chia AugScheme) via digstore-crypto.
        let (sk, pk) = digstore_crypto::bls_keygen(&[7u8; 32]);
        let store_id = b32(0x11);
        let root = b32(0x22);
        // canonical message order: (root, store_id)
        let msg = push_signing_message(&root, &store_id);
        let sig = digstore_crypto::bls_sign(&sk, &msg);

        assert!(verify_push_signature(&pk, &root, &store_id, &sig));

        // tamper: a different root must not verify under the same signature.
        assert!(!verify_push_signature(&pk, &b32(0x23), &store_id, &sig));
        // tamper: a different store id must not verify either.
        assert!(!verify_push_signature(&pk, &root, &b32(0x12), &sig));
    }

    /// CONVENTIONS C7: re-check the shared push-signing vector here. The message
    /// is byte-identical to `digstore_crypto::push_signing_message(root, store_id)`
    /// and `sign_push` produces a signature `verify_push` (and thus this crate's
    /// `verify_push_signature`) accepts.
    #[test]
    fn c7_shared_vector_parity_with_crypto() {
        let (sk, pk) = digstore_crypto::bls_keygen(&[0xC7u8; 32]);
        let root = Bytes32([0xAB; 32]);
        let store_id = Bytes32([0xCD; 32]);

        // message parity: SHA-256(root || store_id), built once in crypto.
        let expected = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&root.0);
            buf[32..].copy_from_slice(&store_id.0);
            digstore_crypto::sha256(&buf).0
        };
        assert_eq!(push_signing_message(&root, &store_id), expected);

        // signature parity: crypto's sign_push == bls_sign over the same message,
        // and both this crate and crypto's verify accept it.
        let sig = digstore_crypto::sign_push(&sk, &root, &store_id);
        assert_eq!(sig, digstore_crypto::bls_sign(&sk, &expected));
        assert!(verify_push_signature(&pk, &root, &store_id, &sig));
    }
}
