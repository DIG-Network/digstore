//! Shared plumbing for the Wave-B asset commands (`nft`/`collection`/`did`/`offer`).
//!
//! The asset commands all follow the same shape: unlock the wallet seed → talk to coinset
//! ([`ChainReads`]) → select a funding coin → build the spend with a `digstore-chain` builder → sign
//! with the wallet's synthetic key → push via coinset. This module owns the pieces shared across
//! those commands so each command file stays a thin, readable orchestration:
//!
//! * [`unlock_mnemonic`] — the wallet seed → mnemonic (reuses the `commit`/`balance` unlock path);
//! * [`chain_reads`] — the [`ChainReads`] backend (mainnet coinset, or an in-memory mock gated by
//!   `DIGSTORE_ANCHOR_MOCK`);
//! * [`scan_and_select_funding`] — scan the HD wallet and pick an XCH coin to fund a mint/create;
//! * [`push_signed`] — push a signed [`SpendBundle`] and return its tx id;
//! * [`parse_xch_address`] / [`parse_launcher_id`] — input parsing with CLI-friendly errors;
//! * [`dig_uri`] / [`gateway_uri`] — the dig:// + https-fallback URI pair for capsule media (#33).
//!
//! The backend is mock-gated by `DIGSTORE_ANCHOR_MOCK` (the same gate `init`/`commit` use), so the
//! offline integration suite drives the asset BUILD paths (`--dry-run`) and the capsule-media path
//! without any network; the on-chain spend round-trips are additionally covered by the chain crate's
//! `Simulator` tests. A mocked run prints a loud warning ([`warn_if_mocked`]) so it can never be
//! mistaken for a real on-chain spend.

use async_trait::async_trait;
use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use digstore_chain::coinset::{ChainReads, CoinInfo, Coinset};
use digstore_chain::keys::IndexedKeys;
use digstore_chain::wallet::scan_wallet;
use digstore_chain::Result as ChainResult;
use digstore_core::Bytes32 as CoreBytes32;
use zeroize::Zeroizing;

use crate::error::CliError;
use crate::ui::Ui;

/// Unlock the wallet seed and return the mnemonic (the asset builders derive their own
/// [`IndexedKeys`] from it). Reuses the shared unlock path, so a missing seed surfaces as
/// [`CliError::NoSeed`] exactly like `commit`/`balance`.
pub fn unlock_mnemonic(ui: &Ui) -> Result<Zeroizing<String>, CliError> {
    let (_keys, phrase, _gcfg) = crate::ops::wallet::unlock_wallet_phrase(ui)?;
    Ok(phrase)
}

/// The chain-reads backend the asset commands use. Production is mainnet coinset; when
/// `DIGSTORE_ANCHOR_MOCK` is set (the same gate the anchor backend uses) it is an in-memory mock so
/// the offline/CI suite can drive the asset BUILD paths (`--dry-run`) without any network. Returns
/// `(backend, mocked)` so callers can warn loudly on a mocked run (an asset spend must never be
/// mistaken for real, like `init`/`commit`).
pub fn chain_reads() -> (Box<dyn ChainReads>, bool) {
    if std::env::var_os("DIGSTORE_ANCHOR_MOCK").is_some() {
        (Box::new(MockChainReads::default()), true)
    } else {
        (Box::new(Coinset::mainnet()), false)
    }
}

/// In-memory, network-free [`ChainReads`] for the offline asset-command suite. It exposes ONE
/// synthetic XCH funding coin at the ABANDON test wallet's index-0 address (enough to fund a mint's
/// 1-mojo launcher) so `scan_and_select_funding` succeeds, accepts (and drops) any pushed bundle, and
/// returns empties for the reconstruction reads (so `nft list` is empty under the mock). It mirrors
/// the anchor `MockAnchor` so a mocked asset run is fully deterministic.
struct MockChainReads {
    funding_ph: Bytes32,
}

impl Default for MockChainReads {
    fn default() -> Self {
        // The ABANDON test vector's index-0 owner puzzle hash (the seeded mock wallet).
        const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon art";
        let funding_ph = digstore_chain::keys::derive_indexed_keys(ABANDON, 0..1)
            .map(|k| k[0].owner_puzzle_hash)
            .unwrap_or_default();
        Self { funding_ph }
    }
}

#[async_trait]
impl ChainReads for MockChainReads {
    async fn unspent_coins(&self, puzzle_hash: Bytes32) -> ChainResult<Vec<Coin>> {
        // A single large XCH coin at the funding address; empty elsewhere.
        if puzzle_hash == self.funding_ph {
            Ok(vec![Coin::new(
                Bytes32::default(),
                self.funding_ph,
                1_000_000_000_000,
            )])
        } else {
            Ok(vec![])
        }
    }
    async fn unspent_coins_by_hint(&self, _hint: Bytes32) -> ChainResult<Vec<Coin>> {
        Ok(vec![])
    }
    async fn coin_records_by_puzzle_hash(
        &self,
        _puzzle_hash: Bytes32,
        _include_spent: bool,
    ) -> ChainResult<Vec<CoinInfo>> {
        Ok(vec![])
    }
    async fn coin_record(&self, _name: Bytes32) -> ChainResult<Option<CoinInfo>> {
        Ok(None)
    }
    async fn coin_spend(
        &self,
        _coin_id: Bytes32,
        _spent_height: u32,
    ) -> ChainResult<Option<CoinSpend>> {
        Ok(None)
    }
    async fn peak_height(&self) -> ChainResult<u32> {
        Ok(1)
    }
    async fn push(&self, _bundle: SpendBundle) -> ChainResult<()> {
        Ok(())
    }
    async fn estimate_fee(&self, _bundle: &SpendBundle, _target_secs: u64) -> ChainResult<u64> {
        Ok(0)
    }
}

/// Print a loud warning when the asset backend is mocked, so a mocked run is never mistaken for a
/// real on-chain spend. No-op when not mocked, and suppressed in `--json` mode (the command's JSON
/// carries a `"mocked": true` flag instead).
pub fn warn_if_mocked(ui: &Ui, mocked: bool) {
    if mocked {
        ui.line("⚠ ASSET BACKEND MOCKED (DIGSTORE_ANCHOR_MOCK) — nothing is on Chia mainnet");
    }
}

/// Scan the HD wallet over `chain` and return its primary keys plus a single XCH funding coin large
/// enough to cover `need` mojos (the 1-mojo singleton launcher + any fee). The first sufficiently
/// large coin is chosen; mints/creates fund from one coin (the launcher path takes a single parent).
///
/// Errors with [`CliError::InsufficientFunds`] (carrying the wallet address to fund) when no single
/// coin covers `need`.
pub async fn scan_and_select_funding(
    chain: &dyn ChainReads,
    mnemonic: &str,
    need: u64,
) -> Result<(IndexedKeys, Coin), CliError> {
    let scanned = scan_wallet(chain, mnemonic).await.map_err(CliError::from)?;

    // Find the address+coin with the largest single XCH coin >= need.
    let mut best: Option<(&IndexedKeys, Coin)> = None;
    let mut total: u64 = 0;
    for a in &scanned.addrs {
        for c in &a.xch {
            total = total.saturating_add(c.amount);
            if c.amount >= need
                && best
                    .as_ref()
                    .map(|(_, b)| c.amount > b.amount)
                    .unwrap_or(true)
            {
                best = Some((&a.keys, *c));
            }
        }
    }

    match best {
        Some((keys, coin)) => Ok((keys.clone(), coin)),
        None => {
            // No single coin is large enough; report the shortfall against the wallet's address.
            let primary = digstore_chain::keys::derive_indexed_keys(mnemonic, 0..1)
                .map_err(CliError::from)?
                .into_iter()
                .next()
                .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;
            let address = digstore_chain::keys::owner_address(&digstore_chain::keys::WalletKeys {
                synthetic_sk: primary.synthetic_sk.clone(),
                synthetic_pk: primary.synthetic_pk,
                owner_puzzle_hash: primary.owner_puzzle_hash,
            });
            Err(CliError::InsufficientFunds {
                need,
                have: total,
                address,
                asset: "XCH".into(),
            })
        }
    }
}

/// Push a signed [`SpendBundle`] to coinset and return its conventional tx id (the bundle name).
pub async fn push_signed(chain: &dyn ChainReads, bundle: SpendBundle) -> Result<Bytes32, CliError> {
    let tx_id = bundle.name();
    chain.push(bundle).await.map_err(CliError::from)?;
    Ok(tx_id)
}

/// Parse a mainnet `xch1…` address into its 32-byte puzzle hash, with a CLI-friendly error.
pub fn parse_xch_address(address: &str) -> Result<Bytes32, CliError> {
    digstore_chain::send::decode_xch_address(address)
        .map_err(|e| CliError::InvalidArgument(format!("invalid --to address: {e}")))
}

/// Parse a 64-hex launcher/coin id (a leading `0x` is tolerated) into a chain [`Bytes32`].
/// `nft1…`/`did:chia:…` bech32 ids are NOT yet decoded here — pass the hex launcher id (see TODO).
pub fn parse_launcher_id(s: &str) -> Result<Bytes32, CliError> {
    // TODO(#35): accept `nft1…`/`did:chia:…` bech32m ids (decode to launcher id) in addition to hex.
    let raw = hex::decode(s.trim().trim_start_matches("0x"))
        .map_err(|e| CliError::InvalidArgument(format!("not a 64-hex launcher id: {e}")))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| CliError::InvalidArgument("launcher id must be exactly 32 bytes".into()))?;
    Ok(Bytes32::new(arr))
}

/// The permanent `dig://` URI for a resource in a capsule — the PRIMARY media URI (#33).
///
/// `dig://<storeId>:<rootHash>/<resource>` is the rootless-friendly capsule form the DIG Browser /
/// resolver understand. This is the URI a verifier should prefer; [`gateway_uri`] is the https
/// fallback. The capsule identity is a `digstore_core::Bytes32` (the store/root types digstore-core
/// emits).
pub fn dig_uri(store_id: CoreBytes32, root_hash: CoreBytes32, resource: &str) -> String {
    format!(
        "dig://{}:{}/{}",
        store_id.to_hex(),
        root_hash.to_hex(),
        resource
    )
}

/// The https gateway fallback URI for a capsule resource (#33): `<gateway>/urn:dig:chia:…/<resource>`.
/// `gateway_base` is e.g. `https://rpc.dig.net` (no trailing slash needed).
pub fn gateway_uri(
    gateway_base: &str,
    store_id: CoreBytes32,
    root_hash: CoreBytes32,
    resource: &str,
) -> String {
    let base = gateway_base.trim_end_matches('/');
    format!(
        "{base}/urn:dig:chia:{}:{}/{}",
        store_id.to_hex(),
        root_hash.to_hex(),
        resource
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: u8) -> CoreBytes32 {
        CoreBytes32([x; 32])
    }

    #[test]
    fn dig_uri_is_capsule_form() {
        let u = dig_uri(b(0xaa), b(0xbb), "art.png");
        assert!(u.starts_with("dig://"));
        assert!(u.contains(&b(0xaa).to_hex()));
        assert!(u.contains(&b(0xbb).to_hex()));
        assert!(u.ends_with("/art.png"));
    }

    #[test]
    fn gateway_uri_trims_trailing_slash_and_uses_urn() {
        let u = gateway_uri("https://rpc.dig.net/", b(0x11), b(0x22), "art.png");
        assert_eq!(
            u,
            format!(
                "https://rpc.dig.net/urn:dig:chia:{}:{}/art.png",
                b(0x11).to_hex(),
                b(0x22).to_hex()
            )
        );
    }

    #[test]
    fn parse_launcher_id_accepts_0x_prefix_and_plain_hex() {
        let plain = "ab".repeat(32);
        let with0x = format!("0x{plain}");
        assert_eq!(
            parse_launcher_id(&plain).unwrap(),
            parse_launcher_id(&with0x).unwrap()
        );
    }

    #[test]
    fn parse_launcher_id_rejects_non_hex() {
        assert!(parse_launcher_id("not-hex").is_err());
    }
}
