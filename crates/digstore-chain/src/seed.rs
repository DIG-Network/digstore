//! BIP-39 mnemonic handling and encrypted seed storage.

use crate::error::{ChainError, Result};
use bip39::Mnemonic;
use zeroize::Zeroizing;

/// Validates a BIP-39 mnemonic phrase and returns it normalized.
///
/// Accepts 12/15/18/21/24-word English mnemonics with a valid checksum.
pub fn validate_mnemonic(phrase: &str) -> Result<Zeroizing<String>> {
    let m = Mnemonic::parse(phrase.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    Ok(Zeroizing::new(m.to_string()))
}

/// Generates a new BIP-39 mnemonic with the given word count (12/15/18/21/24).
pub fn generate_mnemonic(word_count: usize) -> Result<Zeroizing<String>> {
    let m = Mnemonic::generate(word_count)
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    Ok(Zeroizing::new(m.to_string()))
}

#[cfg(test)]
mod mnemonic_tests {
    use super::*;

    const VALID_24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn valid_24_word_parses() {
        let m = validate_mnemonic(VALID_24).unwrap();
        assert_eq!(m.split_whitespace().count(), 24);
    }

    #[test]
    fn invalid_word_rejected() {
        let bad = VALID_24.replace("art", "zzzzzz");
        assert!(matches!(validate_mnemonic(&bad), Err(ChainError::InvalidMnemonic(_))));
    }

    #[test]
    fn bad_checksum_rejected() {
        // 24 valid words but wrong checksum (last word swapped to another valid word).
        let bad = VALID_24.replace("art", "abandon");
        assert!(matches!(validate_mnemonic(&bad), Err(ChainError::InvalidMnemonic(_))));
    }

    #[test]
    fn generate_24_round_trips() {
        let m = generate_mnemonic(24).unwrap();
        assert_eq!(m.split_whitespace().count(), 24);
        // Generated mnemonic must itself validate.
        assert!(validate_mnemonic(&m).is_ok());
    }

    #[test]
    fn generate_rejects_bad_word_count() {
        assert!(generate_mnemonic(13).is_err());
    }
}

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};

const SEED_FILE_VERSION: u8 = 1;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

// Argon2id parameters (matches dig_library): t=3, m=64 MiB, p=4, 32-byte output.
const ARGON_M_COST: u32 = 65536; // KiB
const ARGON_T_COST: u32 = 3;
const ARGON_P_COST: u32 = 4;

/// Encrypted mnemonic blob: `version(1) ‖ salt(32) ‖ nonce(12) ‖ ciphertext+tag`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSeed {
    pub version: u8,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>, // includes the 16-byte GCM tag
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(ARGON_M_COST, ARGON_T_COST, ARGON_P_COST, Some(KEY_LEN))
        .map_err(|e| ChainError::Crypto(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut *key)
        .map_err(|e| ChainError::Crypto(e.to_string()))?;
    Ok(key)
}

/// Encrypts a (validated) mnemonic phrase under a passphrase.
pub fn encrypt_seed(phrase: &str, passphrase: &str) -> Result<EncryptedSeed> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| ChainError::Crypto(e.to_string()))?;
    getrandom::getrandom(&mut nonce).map_err(|e| ChainError::Crypto(e.to_string()))?;

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|e| ChainError::Crypto(e.to_string()))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), phrase.as_bytes())
        .map_err(|_| ChainError::Crypto("AES-GCM encrypt failed".into()))?;

    Ok(EncryptedSeed { version: SEED_FILE_VERSION, salt, nonce, ciphertext })
}

/// Decrypts an `EncryptedSeed` back to the mnemonic phrase.
pub fn decrypt_seed(enc: &EncryptedSeed, passphrase: &str) -> Result<Zeroizing<String>> {
    let key = derive_key(passphrase, &enc.salt)?;
    let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|e| ChainError::Crypto(e.to_string()))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&enc.nonce), enc.ciphertext.as_ref())
        .map_err(|_| ChainError::Decrypt)?;
    let phrase = String::from_utf8(plain).map_err(|_| ChainError::Decrypt)?;
    Ok(Zeroizing::new(phrase))
}

impl EncryptedSeed {
    /// Serializes to the on-disk byte layout.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + SALT_LEN + NONCE_LEN + self.ciphertext.len());
        out.push(self.version);
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Parses the on-disk byte layout.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let min = 1 + SALT_LEN + NONCE_LEN + 16; // +16 for the GCM tag
        if bytes.len() < min {
            return Err(ChainError::MalformedSeedFile(format!(
                "too short: {} bytes (need >= {})",
                bytes.len(),
                min
            )));
        }
        let version = bytes[0];
        if version != SEED_FILE_VERSION {
            return Err(ChainError::MalformedSeedFile(format!(
                "unsupported version {version}"
            )));
        }
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        salt.copy_from_slice(&bytes[1..1 + SALT_LEN]);
        nonce.copy_from_slice(&bytes[1 + SALT_LEN..1 + SALT_LEN + NONCE_LEN]);
        let ciphertext = bytes[1 + SALT_LEN + NONCE_LEN..].to_vec();
        Ok(EncryptedSeed { version, salt, nonce, ciphertext })
    }
}

#[cfg(test)]
mod crypto_tests {
    use super::*;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn encrypt_decrypt_round_trip() {
        let enc = encrypt_seed(PHRASE, "hunter2").unwrap();
        let dec = decrypt_seed(&enc, "hunter2").unwrap();
        assert_eq!(&*dec, PHRASE);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let enc = encrypt_seed(PHRASE, "hunter2").unwrap();
        assert!(matches!(decrypt_seed(&enc, "wrong"), Err(ChainError::Decrypt)));
    }

    #[test]
    fn bytes_round_trip() {
        let enc = encrypt_seed(PHRASE, "pw").unwrap();
        let bytes = enc.to_bytes();
        let parsed = EncryptedSeed::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, enc);
        assert_eq!(&*decrypt_seed(&parsed, "pw").unwrap(), PHRASE);
    }

    #[test]
    fn from_bytes_rejects_truncated() {
        assert!(matches!(
            EncryptedSeed::from_bytes(&[1u8; 10]),
            Err(ChainError::MalformedSeedFile(_))
        ));
    }

    #[test]
    fn from_bytes_rejects_bad_version() {
        let enc = encrypt_seed(PHRASE, "pw").unwrap();
        let mut bytes = enc.to_bytes();
        bytes[0] = 9;
        assert!(matches!(
            EncryptedSeed::from_bytes(&bytes),
            Err(ChainError::MalformedSeedFile(_))
        ));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let enc = encrypt_seed(PHRASE, "pw").unwrap();
        let mut bad = enc.clone();
        bad.ciphertext[0] ^= 0x01;
        assert!(matches!(decrypt_seed(&bad, "pw"), Err(ChainError::Decrypt)));
    }
}

use std::path::Path;

/// Writes bytes to `path` with owner-only permissions where the platform
/// supports it (unix `0600`); on Windows the file inherits the (already
/// user-scoped) `~/.dig` directory ACL. Mirrors digstore-cli's
/// `write_secret_file`.
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
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

/// Saves an encrypted seed to `path` (owner-only).
pub fn save_seed(path: &Path, enc: &EncryptedSeed) -> Result<()> {
    write_secret_file(path, &enc.to_bytes())?;
    Ok(())
}

/// Loads an encrypted seed from `path`.
pub fn load_seed(path: &Path) -> Result<EncryptedSeed> {
    if !path.exists() {
        return Err(ChainError::NoSeed(path.display().to_string()));
    }
    let bytes = std::fs::read(path)?;
    EncryptedSeed::from_bytes(&bytes)
}

/// Whether a seed file exists at `path`.
pub fn seed_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod file_tests {
    use super::*;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seed.enc");
        let enc = encrypt_seed(PHRASE, "pw").unwrap();
        save_seed(&path, &enc).unwrap();
        assert!(seed_exists(&path));
        let loaded = load_seed(&path).unwrap();
        assert_eq!(&*decrypt_seed(&loaded, "pw").unwrap(), PHRASE);
    }

    #[test]
    fn load_missing_is_no_seed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.enc");
        assert!(matches!(load_seed(&path), Err(ChainError::NoSeed(_))));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seed.enc");
        save_seed(&path, &encrypt_seed(PHRASE, "pw").unwrap()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}