use digstore_core::SecretSalt;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

/// Fixed HKDF salt domain string for stores (paper §11.1, §11.4).
const HKDF_SALT_DOMAIN: &[u8] = b"digstore-hkdf-salt-v1";
/// Fixed HKDF `info` context for the AES-256-GCM content key (paper §11.1).
const HKDF_INFO: &[u8] = b"digstore-aes-256-gcm-key-v1";

/// Derive the 32-byte AES-256 content key for a resource from its canonical URN.
///
/// `ikm = canonical_urn` bytes. For public stores the salt is
/// `SHA-256(HKDF_SALT_DOMAIN)`. For private stores (paper §11.4) the
/// `SecretSalt` is mixed in: `salt = SHA-256(HKDF_SALT_DOMAIN || secret_salt)`.
/// Output is always 32 bytes. Distinct URNs (and distinct salts) derive
/// distinct keys — the invariant that makes the fixed GCM nonce safe (§11.2).
///
/// The salt is passed by reference (`Option<&SecretSalt>`) per CONVENTIONS C10,
/// so `digstore-cli` borrows its store salt without cloning.
pub fn derive_decryption_key(canonical_urn: &str, secret_salt: Option<&SecretSalt>) -> [u8; 32] {
    let mut salt_hasher = Sha256::new();
    salt_hasher.update(HKDF_SALT_DOMAIN);
    if let Some(SecretSalt(secret)) = secret_salt {
        salt_hasher.update(secret);
    }
    let salt_digest = salt_hasher.finalize();
    let mut salt = [0u8; 32];
    salt.copy_from_slice(&salt_digest);

    let hk = Hkdf::<Sha256>::new(Some(&salt), canonical_urn.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}
