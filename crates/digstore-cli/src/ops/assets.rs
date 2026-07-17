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
//! * [`urn`] / [`gateway_uri`] / [`media_uris`] — the canonical URN + https-fallback URI pair for
//!   capsule media (#33/#663): [`media_uris`] returns `[bare root-pinned URN, https gateway url]`.
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
use digstore_core::{Bytes32 as CoreBytes32, Urn, CHAIN};
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

/// Poll `chain` until `coin_id` appears on chain (confirmed in a block) or `timeout_secs` elapses,
/// returning whether it confirmed. Polls every 10 s; skips the sleep after the final check so a
/// budget under 10 s does a single non-blocking poll. Used to gate one collection-mint batch on the
/// prior batch landing (the next batch spends the DID the prior batch recreated — #231).
pub async fn confirm_coin(
    chain: &dyn ChainReads,
    coin_id: Bytes32,
    timeout_secs: u64,
) -> Result<bool, CliError> {
    let polls = (timeout_secs / 10).max(1);
    for i in 0..polls {
        if chain
            .coin_record(coin_id)
            .await
            .map_err(CliError::from)?
            .is_some()
        {
            return Ok(true);
        }
        if i + 1 < polls {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    }
    Ok(false)
}

/// Parse a mainnet `xch1…` address into its 32-byte puzzle hash, with a CLI-friendly error.
pub fn parse_xch_address(address: &str) -> Result<Bytes32, CliError> {
    digstore_chain::send::decode_xch_address(address)
        .map_err(|e| CliError::InvalidArgument(format!("invalid --to address: {e}")))
}

/// Parse a 64-hex launcher/coin id (a leading `0x` is tolerated) into a chain [`Bytes32`].
/// `nft1…`/`did:chia:…` bech32 ids are NOT yet decoded here — pass the hex launcher id (see TODO).
pub fn parse_launcher_id(s: &str) -> Result<Bytes32, CliError> {
    // TODO(#35): accept `nft1…` bech32m ids (decode to launcher id) in addition to hex. `did:chia:…`
    // is handled by `parse_did_arg` below (#198) at the DID-specific call sites.
    let raw = hex::decode(s.trim().trim_start_matches("0x"))
        .map_err(|e| CliError::InvalidArgument(format!("not a 64-hex launcher id: {e}")))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| CliError::InvalidArgument("launcher id must be exactly 32 bytes".into()))?;
    Ok(Bytes32::new(arr))
}

/// Parse a DID identifier that is EITHER a 64-hex launcher id OR a `did:chia:1…` bech32m address —
/// the form Sage and CNI display DIDs in (#198) — decoding the bech32m form to its 32-byte launcher
/// id via [`digstore_chain::did::decode_bech32_did`]. Use this at every `--did` call site instead of
/// [`parse_launcher_id`] (which stays hex-only — it is also used for non-DID ids like `--nft`).
pub fn parse_did_arg(s: &str) -> Result<Bytes32, CliError> {
    let trimmed = s.trim();
    if trimmed.starts_with("did:chia:") {
        return digstore_chain::did::decode_bech32_did(trimmed)
            .map_err(|e| CliError::InvalidArgument(format!("invalid --did address: {e}")));
    }
    parse_launcher_id(trimmed)
}

/// The canonical **bare root-pinned URN** for a resource in a capsule — the PRIMARY media URI
/// (#663/#686).
///
/// Emits `urn:dig:chia:<storeId>:<root>/<resource>`, the single normative resource-identifier form
/// (`digstore_core::Urn::canonical`). It is root-PINNED because NFT media is immutable content —
/// the URN names the exact capsule generation the on-chain hashes are pinned to. DIG-aware wallets
/// resolve this URN natively (via dig-node / rpc.dig.net); [`gateway_uri`] is the https fallback for
/// legacy wallets. NEVER a `dig://`-prefixed URN (`dig://` is the §21 remote-transport locator, not a
/// resource scheme — the #686 double-scheme bug).
pub fn urn(store_id: CoreBytes32, root_hash: CoreBytes32, resource: &str) -> String {
    Urn {
        chain: CHAIN.to_string(),
        store_id,
        root_hash: Some(root_hash),
        resource_key: Some(resource.to_string()),
    }
    .canonical()
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

/// The NFT1 multi-url backup pair for a capsule resource (#663): the canonical **bare root-pinned
/// URN first** (the primary, DIG-native entry) followed by the **https gateway url** (the fallback
/// for legacy wallets like Sage).
///
/// NFT1 `data_uris`/`metadata_uris` are LISTS that accept multiple backup urls; a minted NFT carries
/// BOTH so a DIG-aware wallet resolves the URN while a legacy wallet uses the https url — the same
/// URN-first ordering chip35/hub/create-dig-app emit. The list stays additive (§5.1): an old reader
/// simply reads whichever entry it understands.
pub fn media_uris(
    store_id: CoreBytes32,
    root_hash: CoreBytes32,
    resource: &str,
    gateway_base: &str,
) -> Vec<String> {
    vec![
        urn(store_id, root_hash, resource),
        gateway_uri(gateway_base, store_id, root_hash, resource),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: u8) -> CoreBytes32 {
        CoreBytes32([x; 32])
    }

    #[test]
    fn urn_is_bare_canonical_root_pinned() {
        let u = urn(b(0xaa), b(0xbb), "art.png");
        // Canonical bare root-pinned URN (#686) — NEVER a `dig://`-prefixed URN.
        assert_eq!(
            u,
            format!(
                "urn:dig:chia:{}:{}/art.png",
                b(0xaa).to_hex(),
                b(0xbb).to_hex()
            )
        );
        assert!(!u.starts_with("dig://"), "must not be dig://-prefixed");
    }

    #[test]
    fn media_uris_are_urn_first_then_https() {
        let uris = media_uris(b(0x11), b(0x22), "art.png", "https://rpc.dig.net");
        // The NFT1 multi-url backup: canonical URN first (primary), https second (fallback).
        assert_eq!(uris.len(), 2, "both the URN and the https url are present");
        assert_eq!(uris[0], urn(b(0x11), b(0x22), "art.png"));
        assert!(uris[0].starts_with("urn:dig:chia:"), "URN is first");
        assert_eq!(
            uris[1],
            gateway_uri("https://rpc.dig.net", b(0x11), b(0x22), "art.png")
        );
        assert!(uris[1].starts_with("https://"), "https gateway is second");
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

    // ---------- #198: parse_did_arg (bech32 did:chia: + hex fallback) ----------

    #[test]
    fn parse_did_arg_accepts_plain_hex_and_0x() {
        let plain = "ab".repeat(32);
        let with0x = format!("0x{plain}");
        assert_eq!(
            parse_did_arg(&plain).unwrap(),
            parse_did_arg(&with0x).unwrap()
        );
    }

    /// dkackman's real bech32m DID (#198) decodes to the same launcher id whether or not it's
    /// wrapped in surrounding whitespace (a shell copy-paste footgun).
    #[test]
    fn parse_did_arg_decodes_bech32_did() {
        let bech32 = "did:chia:1r00z5mnm8j77akw8mzp4talfzfffra86zasur2usvegftkxu0czqqynhn8";
        let launcher = parse_did_arg(bech32).unwrap();
        assert_eq!(launcher, parse_did_arg(&format!("  {bech32}  ")).unwrap());
        // Cross-check against the underlying chain-crate decoder directly.
        assert_eq!(
            launcher,
            digstore_chain::did::decode_bech32_did(bech32).unwrap()
        );
    }

    #[test]
    fn parse_did_arg_rejects_malformed_bech32_did() {
        assert!(parse_did_arg("did:chia:not-valid-bech32").is_err());
    }
}
