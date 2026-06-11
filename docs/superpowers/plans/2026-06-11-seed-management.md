# Seed Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add encrypted BIP-39 seed management to digstore — first-run import/generate of a mnemonic, Argon2id+AES-256-GCM encryption to `~/.dig/seed.enc`, a cached-unlock session, and `seed`/`lock` CLI commands.

**Architecture:** A new standalone crate `digstore-chain` holds all seed crypto + `~/.dig` I/O as pure, UI-agnostic functions (no CLI dependency, fully unit-testable). The `digstore-cli` crate adds a hidden-input method, four global commands (`seed import`, `seed generate`, `seed status`, `lock`), and routes them before workspace discovery since they need no store.

**Tech Stack:** Rust. `bip39` (mnemonic), `argon2` (KDF), `aes-gcm` (AEAD), `zeroize` (secret hygiene), `getrandom` (salt/nonce), `serde`/`toml` (config), `rpassword` (hidden passphrase entry), `dirs` (home dir). Commands stay synchronous.

**Scope note:** This is subsystem 1 of 2 from `docs/superpowers/specs/2026-06-11-onchain-anchoring-design.md`. Onchain anchoring (subsystem 2) is a separate plan that extends `digstore-chain` with `coinset.rs`/`wallet.rs`/`anchor.rs` and consumes the unlock helper built here.

---

## File Structure

**New crate `crates/digstore-chain/`:**
- `Cargo.toml` — crate manifest + deps.
- `src/lib.rs` — module decls + public re-exports.
- `src/error.rs` — `ChainError` enum.
- `src/config.rs` — `dig_home()` resolution + `GlobalConfig` (`~/.dig/config.toml`) + path helpers.
- `src/seed.rs` — mnemonic validate/generate, `EncryptedSeed` encrypt/decrypt + byte format, `save_seed`/`load_seed`.
- `src/unlock.rs` — `Session` cached-unlock (write/read/clear, TTL).

**Modified in `crates/digstore-cli/`:**
- `Cargo.toml` — add `digstore-chain` (path) + `rpassword` + `zeroize`.
- `src/error.rs` — add `NoSeed`, `BadPassphrase`, `InvalidMnemonic(String)` variants + hints.
- `src/ui/mod.rs` — add `prompt_password`.
- `src/cli.rs` — add `Seed(SeedArgs)` + `Lock(LockArgs)` to `Command`.
- `src/commands/mod.rs` — `mod seed; mod lock;` + dispatch routing.
- `src/commands/seed.rs` — NEW handler (import/generate/status).
- `src/commands/lock.rs` — NEW handler.

**Modified at workspace root:**
- `Cargo.toml` — add `crates/digstore-chain` to `members`.

**Tests:**
- Unit tests inline (`#[cfg(test)]`) in each `digstore-chain` source file.
- `crates/digstore-cli/tests/seed_cmd.rs` — `assert_cmd` integration tests.

---

## Task 1: Scaffold the `digstore-chain` crate

**Files:**
- Create: `crates/digstore-chain/Cargo.toml`
- Create: `crates/digstore-chain/src/lib.rs`
- Create: `crates/digstore-chain/src/error.rs`
- Modify: `Cargo.toml` (workspace `members`)

- [ ] **Step 1: Create the crate manifest**

Create `crates/digstore-chain/Cargo.toml`:

```toml
[package]
name = "digstore-chain"
version.workspace = true
edition = "2021"
license.workspace = true

[dependencies]
bip39 = { version = "2", features = ["rand"] }
argon2 = "0.5"
aes-gcm = "0.10"
zeroize = { version = "1", features = ["zeroize_derive"] }
getrandom = "0.2"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
hex = "0.4"
thiserror = "1"
dirs = "5"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create the error type**

Create `crates/digstore-chain/src/error.rs`:

```rust
//! Error type for seed/chain operations.

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("no seed found at {0}")]
    NoSeed(String),
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
    #[error("decryption failed (wrong passphrase or corrupt seed file)")]
    Decrypt,
    #[error("malformed seed file: {0}")]
    MalformedSeedFile(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, ChainError>;
```

- [ ] **Step 3: Create the lib root**

Create `crates/digstore-chain/src/lib.rs`:

```rust
//! Seed management and (later) Chia anchoring for digstore.

pub mod config;
pub mod error;
pub mod seed;
pub mod unlock;

pub use error::{ChainError, Result};
```

This will not compile yet (modules `config`, `seed`, `unlock` are created in later tasks). Create empty stubs so Task 1 builds on its own:

Create `crates/digstore-chain/src/config.rs` with `// implemented in Task 2`
Create `crates/digstore-chain/src/seed.rs` with `// implemented in Task 3`
Create `crates/digstore-chain/src/unlock.rs` with `// implemented in Task 6`

- [ ] **Step 4: Register the crate in the workspace**

Modify root `Cargo.toml` — add `"crates/digstore-chain"` to the `members` array (append before the closing `]`).

- [ ] **Step 5: Build to verify the crate compiles**

Run: `cargo build -p digstore-chain`
Expected: PASS (empty crate compiles).

- [ ] **Step 6: Commit**

```bash
git add crates/digstore-chain Cargo.toml
git commit -m "feat(chain): scaffold digstore-chain crate"
```

---

## Task 2: Global config + `~/.dig` path resolution

**Files:**
- Modify: `crates/digstore-chain/src/config.rs`
- Test: inline `#[cfg(test)]` in the same file

- [ ] **Step 1: Write the failing tests**

Replace `crates/digstore-chain/src/config.rs` contents:

```rust
//! Global digstore config and `~/.dig` path resolution.

use crate::error::{ChainError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default coinset.org endpoint (used by the anchoring subsystem).
pub const DEFAULT_COINSET_URL: &str = "https://api.coinset.org";
/// Default cached-unlock TTL in seconds (1 hour).
pub const DEFAULT_UNLOCK_TTL: u64 = 3600;

/// Resolves the global `~/.dig` directory.
///
/// Honors the `DIGSTORE_HOME` environment variable (used by tests and for
/// relocating the home dir); otherwise `<home>/.dig`.
pub fn dig_home() -> Result<PathBuf> {
    if let Some(over) = std::env::var_os("DIGSTORE_HOME") {
        return Ok(PathBuf::from(over));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| ChainError::Config("could not resolve home directory".into()))?;
    Ok(home.join(".dig"))
}

pub fn seed_path(home: &Path) -> PathBuf {
    home.join("seed.enc")
}
pub fn session_path(home: &Path) -> PathBuf {
    home.join("session")
}
pub fn config_path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub coinset_url: String,
    pub unlock_ttl: u64,
    pub fee: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        GlobalConfig {
            coinset_url: DEFAULT_COINSET_URL.to_string(),
            unlock_ttl: DEFAULT_UNLOCK_TTL,
            fee: 0,
        }
    }
}

impl GlobalConfig {
    /// Loads config from `<home>/config.toml`, or returns defaults if absent.
    pub fn load(home: &Path) -> Result<Self> {
        let path = config_path(home);
        if !path.exists() {
            return Ok(GlobalConfig::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).map_err(|e| ChainError::Config(e.to_string()))
    }

    /// Writes config to `<home>/config.toml`, creating the dir if needed.
    pub fn save(&self, home: &Path) -> Result<()> {
        std::fs::create_dir_all(home)?;
        let text = toml::to_string_pretty(self).map_err(|e| ChainError::Config(e.to_string()))?;
        std::fs::write(config_path(home), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let c = GlobalConfig::default();
        assert_eq!(c.coinset_url, "https://api.coinset.org");
        assert_eq!(c.unlock_ttl, 3600);
        assert_eq!(c.fee, 0);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let c = GlobalConfig::load(dir.path()).unwrap();
        assert_eq!(c, GlobalConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let c = GlobalConfig { coinset_url: "https://example.org".into(), unlock_ttl: 60, fee: 5 };
        c.save(dir.path()).unwrap();
        let loaded = GlobalConfig::load(dir.path()).unwrap();
        assert_eq!(loaded, c);
    }

    #[test]
    fn dig_home_honors_env_override() {
        std::env::set_var("DIGSTORE_HOME", "/tmp/digstore-test-home");
        let h = dig_home().unwrap();
        assert_eq!(h, PathBuf::from("/tmp/digstore-test-home"));
        std::env::remove_var("DIGSTORE_HOME");
    }

    #[test]
    fn path_helpers_join_filenames() {
        let h = Path::new("/x/.dig");
        assert_eq!(seed_path(h), PathBuf::from("/x/.dig/seed.enc"));
        assert_eq!(session_path(h), PathBuf::from("/x/.dig/session"));
        assert_eq!(config_path(h), PathBuf::from("/x/.dig/config.toml"));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p digstore-chain config::`
Expected: PASS (5 tests). (Implementation and tests were written together; the "failing" state would only occur if the module were empty.)

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-chain/src/config.rs
git commit -m "feat(chain): global config + ~/.dig path resolution"
```

---

## Task 3: Mnemonic validation + generation

**Files:**
- Modify: `crates/digstore-chain/src/seed.rs`

- [ ] **Step 1: Write the mnemonic functions + tests**

Put this at the top of `crates/digstore-chain/src/seed.rs` (the encrypt/decrypt code is appended in Task 4):

```rust
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p digstore-chain mnemonic_tests`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-chain/src/seed.rs
git commit -m "feat(chain): BIP-39 mnemonic validate + generate"
```

---

## Task 4: Encrypt / decrypt the seed (Argon2id + AES-256-GCM)

**Files:**
- Modify: `crates/digstore-chain/src/seed.rs` (append)

- [ ] **Step 1: Append the encryption code + tests**

Append to `crates/digstore-chain/src/seed.rs`:

```rust
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

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let params = Params::new(ARGON_M_COST, ARGON_T_COST, ARGON_P_COST, Some(KEY_LEN))
        .map_err(|e| ChainError::Crypto(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
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
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| ChainError::Crypto(e.to_string()))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), phrase.as_bytes())
        .map_err(|_| ChainError::Crypto("AES-GCM encrypt failed".into()))?;

    Ok(EncryptedSeed { version: SEED_FILE_VERSION, salt, nonce, ciphertext })
}

/// Decrypts an `EncryptedSeed` back to the mnemonic phrase.
pub fn decrypt_seed(enc: &EncryptedSeed, passphrase: &str) -> Result<Zeroizing<String>> {
    let key = derive_key(passphrase, &enc.salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| ChainError::Crypto(e.to_string()))?;
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
        // And the parsed blob still decrypts.
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
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p digstore-chain crypto_tests`
Expected: PASS (5 tests). Argon2 at 64 MiB makes each test take ~tens of ms — acceptable.

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-chain/src/seed.rs
git commit -m "feat(chain): Argon2id+AES-256-GCM seed encryption"
```

---

## Task 5: Persist the seed file with owner-only permissions

**Files:**
- Modify: `crates/digstore-chain/src/seed.rs` (append)

- [ ] **Step 1: Append save/load + a secret-file writer + tests**

Append to `crates/digstore-chain/src/seed.rs`:

```rust
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p digstore-chain file_tests`
Expected: PASS (3 tests on unix, 2 on Windows).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-chain/src/seed.rs
git commit -m "feat(chain): persist encrypted seed owner-only"
```

---

## Task 6: Cached-unlock session

**Files:**
- Modify: `crates/digstore-chain/src/unlock.rs`

- [ ] **Step 1: Write the session module + tests**

Replace `crates/digstore-chain/src/unlock.rs`:

```rust
//! Cached-unlock session: stores the decrypted mnemonic in `~/.dig/session`
//! with an absolute expiry, so commands within the TTL skip the passphrase
//! prompt. This trades some security for convenience (an accepted tradeoff);
//! the file is written owner-only and wiped on `lock`/expiry.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

#[derive(Debug, Serialize, Deserialize)]
struct Session {
    /// Absolute expiry, seconds since the unix epoch.
    expires_at: u64,
    phrase: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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

/// Caches the phrase with a TTL (seconds from now).
pub fn write_session(path: &Path, phrase: &str, ttl_secs: u64) -> Result<()> {
    let s = Session { expires_at: now_secs().saturating_add(ttl_secs), phrase: phrase.to_string() };
    let json = serde_json::to_vec(&s).map_err(|e| crate::error::ChainError::Config(e.to_string()))?;
    write_secret_file(path, &json)?;
    Ok(())
}

/// Reads the cached phrase if present and unexpired; otherwise `None`.
/// An expired session file is removed as a side effect.
pub fn read_session(path: &Path) -> Option<Zeroizing<String>> {
    let bytes = std::fs::read(path).ok()?;
    let s: Session = serde_json::from_slice(&bytes).ok()?;
    if now_secs() >= s.expires_at {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(Zeroizing::new(s.phrase))
}

/// Removes the session file (used by `digstore lock`). Idempotent.
pub fn clear_session(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// True if there is a valid (unexpired) session.
pub fn is_unlocked(path: &Path) -> bool {
    read_session(path).is_some()
}

// Test-only helper to write a session with an absolute expiry.
#[cfg(test)]
fn write_session_abs(path: &Path, phrase: &str, expires_at: u64) -> Result<()> {
    let s = Session { expires_at, phrase: phrase.to_string() };
    let json = serde_json::to_vec(&s).unwrap();
    write_secret_file(path, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_within_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session(&path, "my phrase", 3600).unwrap();
        assert_eq!(read_session(&path).as_deref(), Some("my phrase"));
        assert!(is_unlocked(&path));
    }

    #[test]
    fn expired_session_returns_none_and_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session_abs(&path, "old", 1).unwrap(); // expires_at = 1 (1970)
        assert!(read_session(&path).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn clear_session_removes_file_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session(&path, "x", 3600).unwrap();
        clear_session(&path).unwrap();
        assert!(!path.exists());
        clear_session(&path).unwrap(); // second call: no error
    }
}
```

- [ ] **Step 2: Add the `serde_json` dependency**

`unlock.rs` uses `serde_json`. Add to `crates/digstore-chain/Cargo.toml` `[dependencies]`:

```toml
serde_json = "1"
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p digstore-chain unlock::`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/digstore-chain/src/unlock.rs crates/digstore-chain/Cargo.toml
git commit -m "feat(chain): cached-unlock session with TTL"
```

---

## Task 7: CLI error variants + hints

**Files:**
- Modify: `crates/digstore-cli/src/error.rs`

- [ ] **Step 1: Add the variants**

In `crates/digstore-cli/src/error.rs`, add three variants to the `CliError` enum (before `Other`):

```rust
    #[error("no seed found; run `digstore seed import` or `digstore seed generate`")]
    NoSeed,
    #[error("wrong passphrase")]
    BadPassphrase,
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
```

- [ ] **Step 2: Add hints**

In the `hint()` match arm, add:

```rust
        CliError::NoSeed => Some("run `digstore seed import` to set up your seed".into()),
        CliError::BadPassphrase => Some("re-run and enter the correct passphrase".into()),
        CliError::InvalidMnemonic(_) => Some("check the word list and word count (12/24)".into()),
```

- [ ] **Step 3: Map `ChainError` → `CliError`**

Add a `From<digstore_chain::ChainError>` impl at the end of `crates/digstore-cli/src/error.rs`:

```rust
impl From<digstore_chain::ChainError> for CliError {
    fn from(e: digstore_chain::ChainError) -> Self {
        use digstore_chain::ChainError as C;
        match e {
            C::NoSeed(_) => CliError::NoSeed,
            C::Decrypt => CliError::BadPassphrase,
            C::InvalidMnemonic(m) => CliError::InvalidMnemonic(m),
            other => CliError::Other(anyhow::anyhow!(other.to_string())),
        }
    }
}
```

- [ ] **Step 4: Add the dependency**

In `crates/digstore-cli/Cargo.toml` `[dependencies]`, add:

```toml
digstore-chain = { path = "../digstore-chain" }
rpassword = "7"
zeroize = "1"
```

- [ ] **Step 5: Build to verify it compiles**

Run: `cargo build -p digstore-cli`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/digstore-cli/src/error.rs crates/digstore-cli/Cargo.toml
git commit -m "feat(cli): seed error variants + ChainError mapping"
```

---

## Task 8: Hidden passphrase input on `Ui`

**Files:**
- Modify: `crates/digstore-cli/src/ui/mod.rs`

- [ ] **Step 1: Add `prompt_password`**

In `crates/digstore-cli/src/ui/mod.rs`, add a method to the `impl Ui` block (next to `prompt_line`):

```rust
    /// Prompts for a passphrase with hidden (non-echoed) input.
    /// Returns `None` when not attached to an interactive terminal.
    pub fn prompt_password(&self, prompt: &str) -> Option<String> {
        if !self.interactive() {
            return None;
        }
        rpassword::prompt_password(format!("{prompt}: ")).ok()
    }
```

- [ ] **Step 2: Build**

Run: `cargo build -p digstore-cli`
Expected: PASS (`rpassword` was added in Task 7).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-cli/src/ui/mod.rs
git commit -m "feat(cli): hidden passphrase prompt"
```

---

## Task 9: `seed` + `lock` command definitions

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs`

- [ ] **Step 1: Add args structs**

In `crates/digstore-cli/src/cli.rs`, add near the other `Args` structs:

```rust
#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    #[command(subcommand)]
    pub action: SeedAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum SeedAction {
    /// Import an existing BIP-39 mnemonic.
    Import {
        /// Provide the mnemonic non-interactively (otherwise prompted).
        #[arg(long)]
        mnemonic: Option<String>,
    },
    /// Generate a new BIP-39 mnemonic.
    Generate {
        /// Word count (12/15/18/21/24).
        #[arg(long, default_value_t = 24)]
        words: usize,
    },
    /// Show whether a seed exists and is currently unlocked.
    Status,
}

#[derive(Debug, clap::Args)]
pub struct LockArgs {}
```

- [ ] **Step 2: Add `Command` variants**

In the `Command` enum, add:

```rust
    /// Manage the encrypted wallet seed in ~/.dig.
    Seed(SeedArgs),
    /// Lock the seed (clear the cached-unlock session).
    Lock(LockArgs),
```

- [ ] **Step 3: Build**

Run: `cargo build -p digstore-cli`
Expected: FAIL — `dispatch` is not exhaustive (no arm for `Seed`/`Lock`). That's expected; Task 10 wires them.

- [ ] **Step 4: Commit**

```bash
git add crates/digstore-cli/src/cli.rs
git commit -m "feat(cli): define seed + lock commands"
```

---

## Task 10: `seed` + `lock` handlers + dispatch routing

**Files:**
- Create: `crates/digstore-cli/src/commands/seed.rs`
- Create: `crates/digstore-cli/src/commands/lock.rs`
- Modify: `crates/digstore-cli/src/commands/mod.rs`

- [ ] **Step 1: Write the `lock` handler**

Create `crates/digstore-cli/src/commands/lock.rs`:

```rust
use crate::error::CliError;
use crate::ui::Ui;
use digstore_chain::{config, unlock};

pub fn run(ui: &Ui) -> Result<(), CliError> {
    let home = config::dig_home().map_err(CliError::from)?;
    unlock::clear_session(&config::session_path(&home)).map_err(CliError::from)?;
    ui.success("seed locked");
    Ok(())
}
```

- [ ] **Step 2: Write the `seed` handler**

Create `crates/digstore-cli/src/commands/seed.rs`:

```rust
use crate::cli::{SeedAction, SeedArgs};
use crate::error::CliError;
use crate::ui::Ui;
use digstore_chain::{config, seed, unlock};
use zeroize::Zeroizing;

/// Resolves a passphrase: `DIGSTORE_PASSPHRASE` env wins, else hidden prompt.
fn resolve_passphrase(ui: &Ui, prompt: &str) -> Result<Zeroizing<String>, CliError> {
    if let Some(p) = std::env::var_os("DIGSTORE_PASSPHRASE") {
        return Ok(Zeroizing::new(p.to_string_lossy().into_owned()));
    }
    ui.prompt_password(prompt)
        .map(Zeroizing::new)
        .ok_or(CliError::BadPassphrase)
}

pub fn run(ui: &Ui, args: SeedArgs) -> Result<(), CliError> {
    let home = config::dig_home().map_err(CliError::from)?;
    let cfg = config::GlobalConfig::load(&home).map_err(CliError::from)?;
    let seed_path = config::seed_path(&home);
    let session_path = config::session_path(&home);

    match args.action {
        SeedAction::Import { mnemonic } => {
            let phrase = match mnemonic {
                Some(m) => seed::validate_mnemonic(&m).map_err(CliError::from)?,
                None => {
                    let raw = ui
                        .prompt_line("Enter your BIP-39 mnemonic", "")
                        .ok_or_else(|| CliError::InvalidMnemonic("no input".into()))?;
                    seed::validate_mnemonic(&raw).map_err(CliError::from)?
                }
            };
            let pass = resolve_passphrase(ui, "Set a passphrase to encrypt your seed")?;
            let enc = seed::encrypt_seed(&phrase, &pass).map_err(CliError::from)?;
            seed::save_seed(&seed_path, &enc).map_err(CliError::from)?;
            unlock::write_session(&session_path, &phrase, cfg.unlock_ttl).map_err(CliError::from)?;
            ui.success("seed imported and unlocked");
            Ok(())
        }
        SeedAction::Generate { words } => {
            let phrase = seed::generate_mnemonic(words).map_err(CliError::from)?;
            if !ui.json() {
                ui.line("");
                ui.line("Your new mnemonic — write it down and store it safely:");
                ui.line("");
                ui.line(format!("    {}", &*phrase));
                ui.line("");
            }
            let pass = resolve_passphrase(ui, "Set a passphrase to encrypt your seed")?;
            let enc = seed::encrypt_seed(&phrase, &pass).map_err(CliError::from)?;
            seed::save_seed(&seed_path, &enc).map_err(CliError::from)?;
            unlock::write_session(&session_path, &phrase, cfg.unlock_ttl).map_err(CliError::from)?;
            ui.success("seed generated and unlocked");
            Ok(())
        }
        SeedAction::Status => {
            let exists = seed::seed_exists(&seed_path);
            let unlocked = unlock::is_unlocked(&session_path);
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "seed_exists": exists,
                    "unlocked": unlocked,
                }));
            } else if !exists {
                ui.line("no seed (run `digstore seed import` or `digstore seed generate`)");
            } else if unlocked {
                ui.line("seed: present, unlocked");
            } else {
                ui.line("seed: present, locked");
            }
            Ok(())
        }
    }
}
```

- [ ] **Step 3: Register modules + route in dispatch**

In `crates/digstore-cli/src/commands/mod.rs`, add module declarations near the other `mod` lines:

```rust
mod lock;
mod seed;
```

Then, in `dispatch()`, add routing for the two global commands. Insert these arms into the **first** match (the one that handles `Command::Init` and returns early), alongside the existing workspace-independent arms:

```rust
        Command::Seed(a) => return seed::run(&ui, a),
        Command::Lock(_) => return lock::run(&ui),
```

Finally, in the **second** (store-scoped) match's catch-all `unreachable!` arm, add `Seed`/`Lock` to the list of already-handled variants:

```rust
        Command::Init(_) | Command::Clone(_) | Command::Stores(_) | Command::Use(_)
        | Command::Update(_) | Command::Seed(_) | Command::Lock(_) => {
            unreachable!("handled above")
        }
```

- [ ] **Step 4: Build**

Run: `cargo build -p digstore-cli`
Expected: PASS (dispatch is now exhaustive).

- [ ] **Step 5: Commit**

```bash
git add crates/digstore-cli/src/commands/seed.rs crates/digstore-cli/src/commands/lock.rs crates/digstore-cli/src/commands/mod.rs
git commit -m "feat(cli): seed import/generate/status + lock commands"
```

---

## Task 11: CLI integration tests

**Files:**
- Create: `crates/digstore-cli/tests/seed_cmd.rs`

- [ ] **Step 1: Write the integration tests**

Create `crates/digstore-cli/tests/seed_cmd.rs`:

```rust
//! End-to-end tests for the seed/lock commands. Each test points
//! `DIGSTORE_HOME` at a fresh tempdir so the real `~/.dig` is never touched,
//! and supplies the passphrase via `DIGSTORE_PASSPHRASE` (non-interactive).

use assert_cmd::Command;
use predicates::str::contains;

fn digstore(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.env("DIGSTORE_HOME", home);
    cmd.env("DIGSTORE_PASSPHRASE", "test-pass");
    cmd
}

#[test]
fn status_reports_no_seed_initially() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("no seed"));
}

#[test]
fn generate_then_status_unlocked() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path()).args(["seed", "generate"]).assert().success();
    assert!(home.path().join("seed.enc").exists());
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, unlocked"));
}

#[test]
fn lock_then_status_locked() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path()).args(["seed", "generate"]).assert().success();
    digstore(home.path()).args(["lock"]).assert().success();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, locked"));
}

#[test]
fn import_known_mnemonic_round_trips() {
    let home = tempfile::tempdir().unwrap();
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    digstore(home.path())
        .args(["seed", "import", "--mnemonic", PHRASE])
        .assert()
        .success();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, unlocked"));
}

#[test]
fn import_rejects_bad_mnemonic() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "import", "--mnemonic", "not a real mnemonic at all"])
        .assert()
        .failure()
        .stderr(contains("invalid mnemonic"));
}
```

- [ ] **Step 2: Ensure dev-deps exist**

Confirm `crates/digstore-cli/Cargo.toml` has `[dev-dependencies]` with `assert_cmd`, `predicates`, and `tempfile`. If any are missing, add:

```toml
[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p digstore-cli --test seed_cmd`
Expected: PASS (5 tests). Note: the test binary build requires the guest wasm (digstore-cli build script, contract D6). If the build script errors, first run `cargo build -p digstore-guest --target wasm32-unknown-unknown --release`, then re-run.

- [ ] **Step 4: Commit**

```bash
git add crates/digstore-cli/tests/seed_cmd.rs crates/digstore-cli/Cargo.toml
git commit -m "test(cli): seed/lock command integration tests"
```

---

## Task 12: Docs + full verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document the commands**

In `README.md`, add a short "Wallet seed" subsection under the install/usage area:

```markdown
### Wallet seed

digstore keeps an encrypted BIP-39 seed in `~/.dig/seed.enc`.

- `digstore seed generate` — create a new mnemonic (shown once; back it up).
- `digstore seed import` — import an existing mnemonic.
- `digstore seed status` — show whether a seed exists and is unlocked.
- `digstore lock` — clear the cached-unlock session.

The seed is encrypted with a passphrase (Argon2id + AES-256-GCM). After unlock
it is cached for a configurable TTL (`~/.dig/config.toml`); `DIGSTORE_PASSPHRASE`
supplies it non-interactively.
```

- [ ] **Step 2: Full workspace build + test**

Run: `cargo build -p digstore-guest --target wasm32-unknown-unknown --release`
Then: `cargo test -p digstore-chain -p digstore-cli`
Expected: all tests PASS.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document seed commands"
```

---

## Self-Review

**Spec coverage** (against `2026-06-11-onchain-anchoring-design.md`, seed-management portions):
- `~/.dig/seed.enc` format `version‖salt‖nonce‖ciphertext+tag` — Task 4 (`EncryptedSeed::to_bytes`). ✔
- Argon2id (t=3, m=64MiB, p=4) → AES-256-GCM — Task 4. ✔
- `~/.dig/config.toml` (coinset_url, unlock_ttl, fee) — Task 2. ✔
- `~/.dig/session` cached unlock + TTL — Task 6. ✔
- Owner-only perms — Tasks 5/6. ✔
- First-run import **or** generate — Task 10. ✔
- `DIGSTORE_PASSPHRASE` non-interactive override — Task 10. ✔
- `seed import|generate|status`, `lock` commands — Tasks 9/10. ✔
- zeroize of decrypted material — `Zeroizing` throughout. ✔
- *Deferred to Plan 2 (correctly out of scope here):* wallet/mnemonic→key derivation, coinset, anchoring, init/commit integration.

**Placeholder scan:** no TBD/TODO; every code step has complete code. The Task 1 module stubs are explicitly temporary and replaced in Tasks 2/3/6.

**Type consistency:** `EncryptedSeed`, `ChainError` variants (`NoSeed`/`Decrypt`/`InvalidMnemonic`/`MalformedSeedFile`/`Crypto`/`Config`), `config::{dig_home,seed_path,session_path,config_path,GlobalConfig}`, `seed::{validate_mnemonic,generate_mnemonic,encrypt_seed,decrypt_seed,save_seed,load_seed,seed_exists}`, `unlock::{write_session,read_session,clear_session,is_unlocked}` are referenced consistently across tasks. `CliError::{NoSeed,BadPassphrase,InvalidMnemonic}` match between Task 7 (def) and Task 10 (use). `Ui::prompt_password` defined Task 8, used Task 10.

**Known risk:** exact crate APIs (`bip39` 2.x `Mnemonic::generate`/`parse`, `argon2` 0.5 `Params::new` arg order, `aes-gcm` 0.10 `new_from_slice`/`encrypt`) are pinned by the version constraints in Task 1; if a minor API differs, fix at the failing-test step.
