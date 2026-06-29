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
use digstore_chain::wallet::ScannedWallet;
use digstore_chain::Result as ChainResult;
use zeroize::Zeroizing;

use crate::ui::Ui;

/// In-memory, network-free [`ChainAnchor`] for offline/CI testing.
pub struct MockAnchor {
    /// Mojos reported by `balance`.
    pub balance_mojos: u64,
    /// DIG base units reported by `dig_balance`.
    pub dig_base_units: u64,
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
            dig_base_units: 1_000_000_000,
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
    /// - `DIGSTORE_ANCHOR_MOCK_DIG`: u64 DIG base units (default 1_000_000_000).
    /// - `DIGSTORE_ANCHOR_MOCK_TIMEOUT=1`: make `confirm` return `Pending`.
    /// - `DIGSTORE_ANCHOR_MOCK_FAIL_MINT=<msg>`: make `mint_empty_store` fail
    ///   with a chain error carrying `<msg>` (exercises the MintFailed path).
    pub fn from_env() -> Self {
        let balance_mojos = std::env::var("DIGSTORE_ANCHOR_MOCK_BALANCE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1_000_000_000_000);
        let dig_base_units = std::env::var("DIGSTORE_ANCHOR_MOCK_DIG")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1_000_000_000);
        let confirm_pending = std::env::var("DIGSTORE_ANCHOR_MOCK_TIMEOUT")
            .map(|v| v == "1")
            .unwrap_or(false);
        let fail_mint = std::env::var("DIGSTORE_ANCHOR_MOCK_FAIL_MINT").ok();
        MockAnchor {
            balance_mojos,
            dig_base_units,
            confirm_pending,
            fail_mint,
            fail_update: None,
        }
    }
}

#[async_trait]
impl ChainAnchor for MockAnchor {
    async fn scan(&self, _mnemonic: &str) -> ChainResult<ScannedWallet> {
        // The mock returns a synthetic ScannedWallet that carries the configured
        // balance_mojos and dig_base_units via the aggregate accessors.
        use digstore_chain::keys::derive_wallet_keys;
        use digstore_chain::wallet::AddressCoins;
        // Use a fixed test-vector mnemonic to derive stable index-0 keys.
        const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon art";
        let keys = derive_wallet_keys(ABANDON)
            .map_err(|e| ChainError::Chain(format!("mock scan derive: {e}")))?;
        let cat_ph = digstore_chain::cat::dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        // Synthesize coins so xch_balance() == balance_mojos and dig_balance() == dig_base_units.
        let xch_coins = if self.balance_mojos > 0 {
            vec![chia_protocol::Coin::new(
                Bytes32::default(),
                keys.owner_puzzle_hash,
                self.balance_mojos,
            )]
        } else {
            vec![]
        };
        let dig_coins = if self.dig_base_units > 0 {
            vec![chia_protocol::Coin::new(
                Bytes32::default(),
                cat_ph,
                self.dig_base_units,
            )]
        } else {
            vec![]
        };
        let indexed = digstore_chain::keys::IndexedKeys {
            index: 0,
            synthetic_sk: keys.synthetic_sk,
            synthetic_pk: keys.synthetic_pk,
            owner_puzzle_hash: keys.owner_puzzle_hash,
        };
        Ok(ScannedWallet {
            addrs: vec![AddressCoins {
                keys: indexed,
                xch: xch_coins,
                dig: dig_coins,
            }],
        })
    }

    async fn balance(&self, w: &ScannedWallet) -> ChainResult<u64> {
        Ok(w.xch_balance())
    }

    async fn dig_balance(&self, w: &ScannedWallet) -> ChainResult<u64> {
        Ok(w.dig_balance())
    }

    async fn mint_empty_store(
        &self,
        _w: &ScannedWallet,
        _label: Option<String>,
        _description: Option<String>,
        _fee: u64,
    ) -> ChainResult<MintOutcome> {
        if let Some(msg) = &self.fail_mint {
            return Err(ChainError::Chain(msg.clone()));
        }
        Ok(MintOutcome {
            launcher_id: random_bytes32(),
            coin_id: random_bytes32(),
            tx_id: random_bytes32(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_root(
        &self,
        _launcher_id: Bytes32,
        _new_root: Bytes32,
        _label: Option<String>,
        _description: Option<String>,
        _w: &ScannedWallet,
        _fee: u64,
        _dig_amount: u64,
    ) -> ChainResult<UpdateOutcome> {
        if let Some(msg) = &self.fail_update {
            return Err(ChainError::Chain(msg.clone()));
        }
        Ok(UpdateOutcome {
            new_coin_id: random_bytes32(),
            tx_id: random_bytes32(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_root_writer(
        &self,
        _launcher_id: Bytes32,
        _new_root: Bytes32,
        _label: Option<String>,
        _description: Option<String>,
        _writer: &digstore_chain::keys::WalletKeys,
        _w: &ScannedWallet,
        _fee: u64,
        _dig_amount: u64,
    ) -> ChainResult<UpdateOutcome> {
        // The mock treats a writer-authorized advance like an owner one (it does no
        // on-chain validation); the writer authorization itself is proven on the
        // Simulator in `digstore_chain::singleton` tests.
        if let Some(msg) = &self.fail_update {
            return Err(ChainError::Chain(msg.clone()));
        }
        Ok(UpdateOutcome {
            new_coin_id: random_bytes32(),
            tx_id: random_bytes32(),
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

/// The shared "anchor gate" used by both `init` and `commit`: unlock the wallet
/// seed (→ [`WalletKeys`] + mnemonic), build the (mock or real) anchor backend,
/// warn loudly if mocked, and surface the configured `fee`. Returns
/// `(keys, mnemonic, anchor, mocked, fee)`. A missing seed surfaces as
/// [`CliError::NoSeed`] from `unlock_wallet_phrase`.
#[allow(clippy::type_complexity)]
pub fn prepare_anchor(
    ui: &Ui,
) -> Result<
    (
        WalletKeys,
        Zeroizing<String>,
        Box<dyn ChainAnchor>,
        bool,
        u64,
    ),
    crate::error::CliError,
> {
    let (keys, phrase, gcfg) = crate::ops::wallet::unlock_wallet_phrase(ui)?;
    let (anchor, mocked) = build_anchor();
    warn_if_mocked(ui, mocked);
    Ok((keys, phrase, anchor, mocked, gcfg.fee))
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

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon art";

    async fn dummy_wallet() -> ScannedWallet {
        // The mock ignores the wallet contents; we just need a valid ScannedWallet.
        MockAnchor::default().scan(ABANDON).await.unwrap()
    }

    #[tokio::test]
    async fn mint_returns_nondefault_and_unique_ids() {
        let m = MockAnchor::default();
        let w = dummy_wallet().await;
        let a = m.mint_empty_store(&w, None, None, 0).await.unwrap();
        let b = m.mint_empty_store(&w, None, None, 0).await.unwrap();
        assert_ne!(a.launcher_id, Bytes32::default());
        assert_ne!(a.launcher_id, b.launcher_id, "two mints must differ");
        assert_ne!(a.coin_id, b.coin_id);
        // tx_id is populated (the conventional bundle-hash tx id) and unique.
        assert_ne!(a.tx_id, Bytes32::default());
        assert_ne!(a.tx_id, b.tx_id);
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
        let w = dummy_wallet().await;
        let err = m.mint_empty_store(&w, None, None, 0).await.unwrap_err();
        assert!(matches!(err, ChainError::Chain(ref s) if s == "boom"));
    }

    #[tokio::test]
    async fn balance_returns_configured() {
        let m = MockAnchor {
            balance_mojos: 12345,
            ..MockAnchor::default()
        };
        let w = m
            .scan(
                "abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon art",
            )
            .await
            .unwrap();
        assert_eq!(m.balance(&w).await.unwrap(), 12345);
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
