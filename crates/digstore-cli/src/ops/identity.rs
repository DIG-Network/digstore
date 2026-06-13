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
//! The public key (hex) is the `<user>` in a `dig://<user>@host/<storeId>` origin;
//! the secret seed is the only persisted material (the BLS key is reconstructed via
//! `from_seed`). `DIG_IDENTITY_DIR` overrides the location (tests / multi-identity).

use std::path::PathBuf;

use digstore_core::{Bytes48, Bytes96};
use digstore_crypto::bls::SecretKey;

use crate::error::CliError;
use crate::ops::store_ops::{random_seed, write_secret_file};

/// Directory holding the user-global identity key: `<config_dir>/dig/`
/// (e.g. `~/.config/dig` on Linux, `%APPDATA%\dig` on Windows). `DIG_IDENTITY_DIR`
/// overrides it.
fn identity_dir() -> Result<PathBuf, CliError> {
    if let Some(d) = std::env::var_os("DIG_IDENTITY_DIR") {
        return Ok(PathBuf::from(d));
    }
    let base = dirs::config_dir().ok_or_else(|| {
        CliError::Other(anyhow::anyhow!(
            "no OS config directory available for the dig identity key"
        ))
    })?;
    Ok(base.join("dig"))
}

fn identity_key_path() -> Result<PathBuf, CliError> {
    Ok(identity_dir()?.join("identity_key.bin"))
}

/// Load the user's identity SEED (the only persisted material), generating +
/// persisting one on first use. Returns the 32-byte seed and the 48-byte G1
/// public key. The seed is `Copy`, so signer closures can capture it and
/// reconstruct the key per call (trivially `Send + Sync`).
pub fn load_or_create_seed() -> Result<([u8; 32], Bytes48), CliError> {
    let path = identity_key_path()?;
    let seed: [u8; 32] = if path.exists() {
        let bytes = std::fs::read(&path).map_err(|e| CliError::Other(e.into()))?;
        bytes.try_into().map_err(|_| {
            CliError::Other(anyhow::anyhow!("identity_key.bin is not a 32-byte seed"))
        })?
    } else {
        let seed = random_seed();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
        }
        write_secret_file(&path, &seed).map_err(|e| CliError::Other(e.into()))?;
        seed
    };
    let pk = SecretKey::from_seed(&seed).public_key().to_bytes();
    Ok((seed, pk))
}

/// The identity public key (48-byte hex), creating the key if absent. This is the
/// `<user>` embedded in a `dig://<user>@host/<storeId>` origin.
pub fn identity_pubkey_hex() -> Result<String, CliError> {
    Ok(load_or_create_seed()?.1.to_hex())
}

/// Build the per-request signer for the remote `DigClient` (paper §21.9): the
/// identity pubkey hex and a BLS signer over the 32-byte canonical request
/// message. The seed is captured and the key reconstructed per call so the
/// closure is trivially `Send + Sync`.
pub fn request_signer(
) -> Result<(String, Box<dyn Fn(&[u8; 32]) -> Bytes96 + Send + Sync>), CliError> {
    let (seed, pk) = load_or_create_seed()?;
    let signer = move |msg: &[u8; 32]| -> Bytes96 {
        digstore_crypto::bls::bls_sign(&SecretKey::from_seed(&seed), msg)
    };
    Ok((pk.to_hex(), Box::new(signer)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_created_then_stable() {
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
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_IDENTITY_DIR", td.path());
        let (pk_hex, sign) = request_signer().unwrap();
        let pk = digstore_crypto::bls::PublicKey::from_bytes(
            &digstore_core::Bytes48::from_hex(&pk_hex).unwrap(),
        )
        .unwrap();
        let store = digstore_core::Bytes32([5u8; 32]);
        let nonce = [1u8; 32];
        let sig = sign(&digstore_crypto::request_signing_message("fetch", &store, 100, &nonce));
        assert!(digstore_crypto::verify_request(
            &pk, "fetch", &store, 100, &nonce, &sig
        ));
        std::env::remove_var("DIG_IDENTITY_DIR");
    }
}
