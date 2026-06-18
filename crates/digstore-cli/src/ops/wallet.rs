//! Seed unlock → wallet keys: the shared bridge from a session/seed file to the
//! `WalletKeys` the anchoring engine signs with. Also owns the passphrase
//! resolution helper shared by `seed` and the anchoring commands.

use digstore_chain::config::{self, GlobalConfig};
use digstore_chain::keys::{derive_wallet_keys, WalletKeys};
use digstore_chain::{seed, unlock};
use zeroize::Zeroizing;

use crate::error::CliError;
use crate::ui::Ui;

/// Resolves a passphrase: `DIGSTORE_PASSPHRASE` env wins, else hidden prompt.
pub(crate) fn resolve_passphrase(ui: &Ui, prompt: &str) -> Result<Zeroizing<String>, CliError> {
    if let Some(p) = std::env::var_os("DIGSTORE_PASSPHRASE") {
        let s = p.into_string().map_err(|_| CliError::BadPassphrase)?;
        return Ok(Zeroizing::new(s));
    }
    ui.prompt_password(prompt)
        .map(Zeroizing::new)
        .ok_or(CliError::BadPassphrase)
}

/// Unlock the wallet: use a live session if present, else decrypt the seed with
/// a passphrase (and refresh the session). Returns the derived keys plus the
/// loaded global config (carries `coinset_url`/`fee`/`unlock_ttl`).
///
/// Errors map cleanly via `?`: a missing seed → [`CliError::NoSeed`], a wrong
/// passphrase → [`CliError::BadPassphrase`] (from `ChainError::Decrypt`).
pub fn unlock_wallet_keys(ui: &Ui) -> Result<(WalletKeys, GlobalConfig), CliError> {
    let (phrase, cfg) = unlock_phrase(ui)?;
    let keys = derive_wallet_keys(&phrase)?;
    Ok((keys, cfg))
}

/// Like [`unlock_wallet_keys`] but also returns the raw mnemonic phrase so the
/// caller can pass it to `anchor.scan(mnemonic)` for a full HD wallet scan.
/// The phrase is zeroized when the `Zeroizing<String>` is dropped.
pub fn unlock_wallet_phrase(
    ui: &Ui,
) -> Result<(WalletKeys, Zeroizing<String>, GlobalConfig), CliError> {
    let (phrase, cfg) = unlock_phrase(ui)?;
    let keys = derive_wallet_keys(&phrase)?;
    Ok((keys, phrase, cfg))
}

/// Internal: resolve the mnemonic phrase + global config from session or encrypted seed.
fn unlock_phrase(ui: &Ui) -> Result<(Zeroizing<String>, GlobalConfig), CliError> {
    let home = config::dig_home()?;
    let cfg = GlobalConfig::load(&home)?;
    let session_path = config::session_path(&home);

    let phrase = match unlock::read_session(&session_path) {
        Some(p) => p,
        None => {
            let seed_path = config::seed_path(&home);
            if !seed::seed_exists(&seed_path) {
                return Err(CliError::NoSeed);
            }
            let enc = seed::load_seed(&seed_path)?;
            // Try an empty passphrase first — seeds created without a passphrase
            // decrypt silently so the user is never prompted unnecessarily.
            let phrase = match seed::decrypt_seed(&enc, "") {
                Ok(p) => p,
                Err(_) => {
                    let pass = resolve_passphrase(ui, "Enter your seed passphrase")?;
                    seed::decrypt_seed(&enc, &pass)?
                }
            };
            // Best-effort session refresh so subsequent commands stay unlocked.
            let _ = unlock::write_session(&session_path, &phrase, cfg.unlock_ttl);
            phrase
        }
    };

    Ok((phrase, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon art";
    // GOLDEN owner puzzle hash for the ABANDON vector.
    const GOLDEN_PH: &str = "d207c1e11fc3b0cd7472e8c7e53c8d2b81709516346c7baa9fbb9070ffccfe89";

    // `DIGSTORE_HOME` is process-global; serialize the env-mutating tests so
    // they cannot interleave when cargo runs them on separate threads.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets `DIGSTORE_HOME` to a tempdir and removes it on drop (mirrors the
    /// pattern in `digstore-chain/src/config.rs` tests). Holds `ENV_LOCK` for
    /// its lifetime so only one test touches the env var at a time.
    struct HomeGuard {
        _td: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl HomeGuard {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let td = tempfile::TempDir::new().unwrap();
            std::env::set_var("DIGSTORE_HOME", td.path());
            HomeGuard {
                _td: td,
                _lock: lock,
            }
        }
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            std::env::remove_var("DIGSTORE_HOME");
        }
    }

    fn test_ui() -> Ui {
        // json=false, quiet=false; not a TTY in test, so prompts return None.
        Ui::resolve(
            crate::ui::ColorChoice::Never,
            false,
            false,
            false,
            false,
            false,
        )
    }

    #[test]
    fn unlock_from_session_derives_golden_keys() {
        let _g = HomeGuard::new();
        let home = config::dig_home().unwrap();
        let session_path = config::session_path(&home);
        std::fs::create_dir_all(&home).unwrap();
        unlock::write_session(&session_path, ABANDON, 3600).unwrap();

        let (keys, cfg) = unlock_wallet_keys(&test_ui()).unwrap();
        assert_eq!(hex::encode(keys.owner_puzzle_hash), GOLDEN_PH);
        assert_eq!(cfg, GlobalConfig::default());
    }

    #[test]
    fn no_session_no_seed_errors_no_seed() {
        let _g = HomeGuard::new();
        // WalletKeys is not Debug, so match instead of `unwrap_err`.
        match unlock_wallet_keys(&test_ui()) {
            Err(CliError::NoSeed) => {}
            Err(other) => panic!("expected NoSeed, got {other:?}"),
            Ok(_) => panic!("expected NoSeed error, got Ok"),
        }
    }
}
