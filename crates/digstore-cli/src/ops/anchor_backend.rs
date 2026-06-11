//! Anchor backend factory and an env-gated in-memory mock.
//!
//! Production code talks to coinset.org via [`CoinsetAnchor::mainnet`]. Tests
//! and CI set `DIGSTORE_ANCHOR_MOCK` to swap in [`MockAnchor`], which never
//! touches the network — and any command using a mock prints a LOUD warning so
//! a mocked run can never be mistaken for real anchoring on Chia mainnet.

use async_trait::async_trait;
use chia_protocol::Bytes32;
use digstore_chain::anchor::{
    ChainAnchor, CoinsetAnchor, ConfirmState, MintOutcome, UpdateOutcome,
};
use digstore_chain::error::ChainError;
use digstore_chain::keys::WalletKeys;
use digstore_chain::Result as ChainResult;

use crate::ui::Ui;

/// In-memory, network-free [`ChainAnchor`] for offline/CI testing.
pub struct MockAnchor {
    /// Mojos reported by `balance`.
    pub balance_mojos: u64,
    /// When true, `confirm` returns `Pending` (simulates a confirmation timeout).
    pub confirm_pending: bool,
    /// `Some(msg)` makes `mint_empty_store` fail with a chain error carrying `msg`.
    pub fail_mint: Option<String>,
    /// `Some(msg)` makes `update_root` fail with a chain error carrying `msg`.
    pub fail_update: Option<String>,
}

impl Default for MockAnchor {
    fn default() -> Self {
        MockAnchor {
            balance_mojos: 1_000_000_000_000,
            confirm_pending: false,
            fail_mint: None,
            fail_update: None,
        }
    }
}

/// A fresh random 32-byte id (real launcher/coin ids are unique per funding
/// coin; two stores minted in one workspace must not collide).
fn random_bytes32() -> Bytes32 {
    let mut arr = [0u8; 32];
    getrandom::getrandom(&mut arr).expect("getrandom");
    Bytes32::new(arr)
}

impl MockAnchor {
    /// Builds a mock from `DIGSTORE_ANCHOR_MOCK_*` env overrides.
    /// - `DIGSTORE_ANCHOR_MOCK_BALANCE`: u64 mojos (default 1_000_000_000_000).
    /// - `DIGSTORE_ANCHOR_MOCK_TIMEOUT=1`: make `confirm` return `Pending`.
    /// - `DIGSTORE_ANCHOR_MOCK_FAIL_MINT=<msg>`: make `mint_empty_store` fail
    ///   with a chain error carrying `<msg>` (exercises the MintFailed path).
    pub fn from_env() -> Self {
        let balance_mojos = std::env::var("DIGSTORE_ANCHOR_MOCK_BALANCE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1_000_000_000_000);
        let confirm_pending = std::env::var("DIGSTORE_ANCHOR_MOCK_TIMEOUT")
            .map(|v| v == "1")
            .unwrap_or(false);
        let fail_mint = std::env::var("DIGSTORE_ANCHOR_MOCK_FAIL_MINT").ok();
        MockAnchor {
            balance_mojos,
            confirm_pending,
            fail_mint,
            fail_update: None,
        }
    }
}

#[async_trait]
impl ChainAnchor for MockAnchor {
    async fn balance(&self, _keys: &WalletKeys) -> ChainResult<u64> {
        Ok(self.balance_mojos)
    }

    async fn mint_empty_store(&self, _keys: &WalletKeys, _fee: u64) -> ChainResult<MintOutcome> {
        if let Some(msg) = &self.fail_mint {
            return Err(ChainError::Chain(msg.clone()));
        }
        Ok(MintOutcome {
            launcher_id: random_bytes32(),
            coin_id: random_bytes32(),
        })
    }

    async fn update_root(
        &self,
        _launcher_id: Bytes32,
        _new_root: Bytes32,
        _keys: &WalletKeys,
        _fee: u64,
    ) -> ChainResult<UpdateOutcome> {
        if let Some(msg) = &self.fail_update {
            return Err(ChainError::Chain(msg.clone()));
        }
        Ok(UpdateOutcome {
            new_coin_id: random_bytes32(),
        })
    }

    async fn confirm(&self, _coin_id: Bytes32, _timeout_secs: u64) -> ChainResult<ConfirmState> {
        if self.confirm_pending {
            Ok(ConfirmState::Pending)
        } else {
            Ok(ConfirmState::Confirmed { height: 1 })
        }
    }
}

/// Builds the anchor backend. Returns `(anchor, mocked)`: when
/// `DIGSTORE_ANCHOR_MOCK` is set the anchor is an in-memory [`MockAnchor`] and
/// `mocked` is `true`; otherwise it is the production [`CoinsetAnchor`] over
/// coinset.org mainnet and `mocked` is `false`.
pub fn build_anchor() -> (Box<dyn ChainAnchor>, bool) {
    if std::env::var_os("DIGSTORE_ANCHOR_MOCK").is_some() {
        (Box::new(MockAnchor::from_env()), true)
    } else {
        (Box::new(CoinsetAnchor::mainnet()), false)
    }
}

/// Prints a loud warning to the user when the anchor is mocked, so a mocked run
/// is never mistaken for real anchoring. No-op when not mocked. In `--json`
/// mode this goes through `Ui::line` (suppressed); the command's JSON carries a
/// `"mocked": true` flag instead.
pub fn warn_if_mocked(ui: &Ui, mocked: bool) {
    if mocked {
        ui.line("⚠ ANCHORING MOCKED (DIGSTORE_ANCHOR_MOCK) — this store is NOT on Chia mainnet");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_keys() -> WalletKeys {
        // The mock ignores keys entirely; derive real ones from a public vector.
        digstore_chain::keys::derive_wallet_keys(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon art",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn mint_returns_nondefault_and_unique_ids() {
        let m = MockAnchor::default();
        let k = dummy_keys();
        let a = m.mint_empty_store(&k, 0).await.unwrap();
        let b = m.mint_empty_store(&k, 0).await.unwrap();
        assert_ne!(a.launcher_id, Bytes32::default());
        assert_ne!(a.launcher_id, b.launcher_id, "two mints must differ");
        assert_ne!(a.coin_id, b.coin_id);
    }

    #[tokio::test]
    async fn confirm_default_is_confirmed() {
        let m = MockAnchor::default();
        let st = m.confirm(Bytes32::default(), 1).await.unwrap();
        assert_eq!(st, ConfirmState::Confirmed { height: 1 });
    }

    #[tokio::test]
    async fn confirm_pending_when_flagged() {
        let m = MockAnchor {
            confirm_pending: true,
            ..MockAnchor::default()
        };
        let st = m.confirm(Bytes32::default(), 1).await.unwrap();
        assert_eq!(st, ConfirmState::Pending);
    }

    #[tokio::test]
    async fn fail_mint_yields_chain_error() {
        let m = MockAnchor {
            fail_mint: Some("boom".into()),
            ..MockAnchor::default()
        };
        let err = m.mint_empty_store(&dummy_keys(), 0).await.unwrap_err();
        assert!(matches!(err, ChainError::Chain(ref s) if s == "boom"));
    }

    #[tokio::test]
    async fn balance_returns_configured() {
        let m = MockAnchor {
            balance_mojos: 12345,
            ..MockAnchor::default()
        };
        assert_eq!(m.balance(&dummy_keys()).await.unwrap(), 12345);
    }

    // `DIGSTORE_ANCHOR_MOCK` is process-global; serialize env-mutating tests.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct MockEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl MockEnvGuard {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("DIGSTORE_ANCHOR_MOCK", "1");
            MockEnvGuard { _lock: lock }
        }
    }
    impl Drop for MockEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("DIGSTORE_ANCHOR_MOCK");
        }
    }

    #[test]
    fn build_anchor_is_mocked_when_env_set() {
        let _g = MockEnvGuard::new();
        let (_anchor, mocked) = build_anchor();
        assert!(mocked, "build_anchor should report mocked when env set");
    }
}
