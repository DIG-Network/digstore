//! The CLI's persistent USER IDENTITY key (paper §21.9).
//!
//! Distinct from a store's per-store `signing_key.bin` (which authorizes pushes to
//! ONE store), the identity key authenticates the *operator* on EVERY `dig://`
//! remote request (fetch / roots / module / content / proof / push / tombstone). It:
//!   * exists independently of any store — so even a `clone` (before any store key
//!     exists) is signed; and
//!   * is stable per machine/user (persisted under the OS config dir), so it is
//!     genuinely "the user", the way an SSH key identifies a git user.
//!
//! The persistence + seed→key derivation + signer construction live in
//! [`digstore_remote::identity`] — the SINGLE source of truth shared with the DIG
//! Browser's `dig-node` sidecar, so the CLI and the browser authenticate to
//! rpc.dig.net identically. This module is a thin wrapper that maps the shared
//! crate's `io::Error` onto [`CliError`] so the CLI surfaces a uniform error type.

use digstore_core::Bytes48;

use crate::error::CliError;

/// Load the user's identity SEED (the only persisted material), generating +
/// persisting one on first use. Returns the 32-byte seed and the 48-byte G1
/// public key. Delegates to [`digstore_remote::identity::load_or_create_seed`].
pub fn load_or_create_seed() -> Result<([u8; 32], Bytes48), CliError> {
    digstore_remote::identity::load_or_create_seed().map_err(|e| CliError::Other(e.into()))
}

/// The identity public key (48-byte hex), creating the key if absent. This is the
/// `<user>` embedded in a `dig://<user>@host/<storeId>` origin.
pub fn identity_pubkey_hex() -> Result<String, CliError> {
    digstore_remote::identity::identity_pubkey_hex().map_err(|e| CliError::Other(e.into()))
}

/// Build the per-request signer for the remote `DigClient` (paper §21.9): the
/// identity pubkey hex and a BLS signer over the 32-byte canonical request
/// message. Delegates to [`digstore_remote::identity::request_signer`].
pub fn request_signer() -> Result<(String, digstore_remote::RequestSignFn), CliError> {
    digstore_remote::identity::request_signer().map_err(|e| CliError::Other(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `DIG_IDENTITY_DIR` is process-global; serialize the tests that mutate it so
    // they cannot clobber each other's setting when run in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn identity_is_created_then_stable() {
        let _g = ENV_LOCK.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_IDENTITY_DIR", td.path());
        let (_seed1, pk1) = load_or_create_seed().unwrap();
        // Re-loading returns the SAME key (persisted, not regenerated).
        let (_seed2, pk2) = load_or_create_seed().unwrap();
        assert_eq!(pk1.to_hex(), pk2.to_hex());
        assert!(td.path().join("identity_key.bin").exists());
        std::env::remove_var("DIG_IDENTITY_DIR");
    }

    #[test]
    fn request_signer_produces_verifiable_signatures() {
        let _g = ENV_LOCK.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_IDENTITY_DIR", td.path());
        let (pk_hex, sign) = request_signer().unwrap();
        let pk = digstore_crypto::bls::PublicKey::from_bytes(
            &digstore_core::Bytes48::from_hex(&pk_hex).unwrap(),
        )
        .unwrap();
        let store = digstore_core::Bytes32([5u8; 32]);
        let nonce = [1u8; 32];
        let sig = sign(&digstore_crypto::request_signing_message(
            "fetch", &store, 100, &nonce,
        ));
        assert!(digstore_crypto::verify_request(
            &pk, "fetch", &store, 100, &nonce, &sig
        ));
        std::env::remove_var("DIG_IDENTITY_DIR");
    }
}
