//! The persistent USER IDENTITY key (paper §21.9), shared by every NATIVE client
//! that authenticates to a §21 remote.
//!
//! Both the digstore CLI and the DIG Browser's `dig-node` sidecar build their
//! per-request signer here, so they authenticate to rpc.dig.net **identically**:
//! the same key location, the same seed→BLS-key derivation, and the same
//! `X-Dig-Identity/-Timestamp/-Nonce/-Auth` stamping performed by [`DigClient`]
//! (this is the single source of truth — neither client re-implements it).
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

use std::path::{Path, PathBuf};

use digstore_core::{Bytes48, Bytes96};
use digstore_crypto::bls::SecretKey;

use crate::client::{RequestIdentity, RequestSignFn};

/// Directory holding the user-global identity key: `<config_dir>/dig/`
/// (e.g. `~/.config/dig` on Linux, `%APPDATA%\dig` on Windows). `DIG_IDENTITY_DIR`
/// overrides it (tests / multi-identity).
fn identity_dir() -> std::io::Result<PathBuf> {
    if let Some(d) = std::env::var_os("DIG_IDENTITY_DIR") {
        return Ok(PathBuf::from(d));
    }
    let base = dirs::config_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no OS config directory available for the dig identity key",
        )
    })?;
    Ok(base.join("dig"))
}

fn identity_key_path() -> std::io::Result<PathBuf> {
    Ok(identity_dir()?.join("identity_key.bin"))
}

/// 32 bytes from the OS CSPRNG. Panics only if the CSPRNG is unavailable, which on
/// any supported platform means the process cannot safely produce key material.
fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed)
        .expect("operating system CSPRNG must be available to generate key material");
    seed
}

/// Write a secret file (the identity seed) with owner-only permissions. On Unix the
/// file is created mode `0600`; on Windows it inherits the user-profile ACL (the
/// identity dir lives under the user's config dir), already restricted to the owner.
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// Load the user's identity SEED (the only persisted material), generating +
/// persisting one on first use. Returns the 32-byte seed and the 48-byte G1
/// public key. The seed is `Copy`, so signer closures can capture it and
/// reconstruct the key per call (trivially `Send + Sync`).
pub fn load_or_create_seed() -> std::io::Result<([u8; 32], Bytes48)> {
    let path = identity_key_path()?;
    let seed: [u8; 32] = if path.exists() {
        let bytes = std::fs::read(&path)?;
        bytes.try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "identity_key.bin is not a 32-byte seed",
            )
        })?
    } else {
        let seed = random_seed();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_secret_file(&path, &seed)?;
        seed
    };
    let pk = SecretKey::from_seed(&seed).public_key().to_bytes();
    Ok((seed, pk))
}

/// The identity public key (48-byte hex), creating the key if absent. This is the
/// `<user>` embedded in a `dig://<user>@host/<storeId>` origin.
pub fn identity_pubkey_hex() -> std::io::Result<String> {
    Ok(load_or_create_seed()?.1.to_hex())
}

/// Build the per-request signer for the remote [`DigClient`] (paper §21.9): the
/// identity pubkey hex and a BLS signer over the 32-byte canonical request
/// message. The seed is captured and the key reconstructed per call so the
/// closure is trivially `Send + Sync`.
pub fn request_signer() -> std::io::Result<(String, RequestSignFn)> {
    let (seed, pk) = load_or_create_seed()?;
    let signer = move |msg: &[u8; 32]| -> Bytes96 {
        digstore_crypto::bls::bls_sign(&SecretKey::from_seed(&seed), msg)
    };
    Ok((pk.to_hex(), Box::new(signer)))
}

/// Build a ready-to-attach [`RequestIdentity`] for `DigClient::with_identity`, so a
/// caller wires authentication in one line:
/// `DigClient::new(url).with_identity(identity::request_identity()?)`.
pub fn request_identity() -> std::io::Result<RequestIdentity> {
    let (pubkey_hex, sign) = request_signer()?;
    Ok(RequestIdentity { pubkey_hex, sign })
}

/// A BLS request signer over a seed that is already in memory (no disk I/O). The
/// seed is captured `Copy` and the key reconstructed per call, so the closure is
/// trivially `Send + Sync`. Use this when a long-lived service (e.g. `dig-node`)
/// loads the seed ONCE at startup via [`load_or_create_seed`] and then mints a
/// fresh [`RequestIdentity`] per `DigClient` (which takes the identity by value).
pub fn signer_from_seed(seed: [u8; 32]) -> RequestSignFn {
    Box::new(move |msg: &[u8; 32]| -> Bytes96 {
        digstore_crypto::bls::bls_sign(&SecretKey::from_seed(&seed), msg)
    })
}

/// Build a [`RequestIdentity`] from an in-memory seed (its G1 pubkey + a
/// [`signer_from_seed`] closure). Companion to [`load_or_create_seed`] for callers
/// that hold the seed and need a fresh identity per request.
pub fn identity_from_seed(seed: [u8; 32]) -> RequestIdentity {
    let pubkey_hex = SecretKey::from_seed(&seed).public_key().to_bytes().to_hex();
    RequestIdentity {
        pubkey_hex,
        sign: signer_from_seed(seed),
    }
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
            "module", &store, 100, &nonce,
        ));
        assert!(digstore_crypto::verify_request(
            &pk, "module", &store, 100, &nonce, &sig
        ));
        std::env::remove_var("DIG_IDENTITY_DIR");
    }
}
