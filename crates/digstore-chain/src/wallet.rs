//! Adaptive HD wallet scan: derive addresses in chunks and aggregate the wallet's
//! XCH + DIG coins across all of them (Sage-style whole-wallet balance), so the CLI
//! no longer sees only index 0.

use crate::cat::dig_cat_puzzle_hash;
use crate::coinset::ChainReads;
use crate::dig::DIG_ASSET_ID;
use crate::error::Result;
use crate::keys::{derive_indexed_keys, IndexedKeys};
use chia_protocol::{Bytes32, Coin};
use datalayer_driver::SecretKey;

const CHUNK: u32 = 50;
const MAX_INDEX: u32 = 500;

/// Number of transactions per page returned by [`wallet_transactions`].
pub const TX_PAGE_SIZE: usize = 50;

/// Per-address coins discovered by the scan.
pub struct AddressCoins {
    pub keys: IndexedKeys,
    pub xch: Vec<Coin>,
    pub dig: Vec<Coin>, // raw DIG CAT coins at this address's DIG ph
}

/// Aggregated result of scanning an HD wallet across multiple addresses.
pub struct ScannedWallet {
    pub addrs: Vec<AddressCoins>,
}

impl ScannedWallet {
    /// Total XCH (mojos) across all scanned addresses.
    pub fn xch_balance(&self) -> u64 {
        self.addrs
            .iter()
            .flat_map(|a| &a.xch)
            .map(|c| c.amount)
            .sum()
    }

    /// Total DIG (base units) across all scanned addresses.
    pub fn dig_balance(&self) -> u64 {
        self.addrs
            .iter()
            .flat_map(|a| &a.dig)
            .map(|c| c.amount)
            .sum()
    }

    /// All synthetic secret keys for kept addresses (for signing).
    pub fn signing_keys(&self) -> Vec<SecretKey> {
        self.addrs
            .iter()
            .map(|a| a.keys.synthetic_sk.clone())
            .collect()
    }
}

/// Scan the HD wallet in chunks of `CHUNK` indices.
///
/// Stops after a full chunk with NO coins (neither XCH nor DIG) anywhere in that
/// chunk.  Index 0 is always kept so a fresh wallet still has a usable address.
/// Caps at `MAX_INDEX` to avoid infinite loops on pathological inputs.
pub async fn scan_wallet(chain: &dyn ChainReads, mnemonic: &str) -> Result<ScannedWallet> {
    let mut addrs = Vec::new();
    let mut start = 0u32;

    while start < MAX_INDEX {
        let end = (start + CHUNK).min(MAX_INDEX);
        let keys = derive_indexed_keys(mnemonic, start..end)?;
        let mut chunk_has_any = false;

        for k in keys {
            let xch = chain.unspent_coins(k.owner_puzzle_hash).await?;
            let dig = chain
                .unspent_coins(dig_cat_puzzle_hash(k.owner_puzzle_hash))
                .await?;

            let has_coins = !xch.is_empty() || !dig.is_empty();
            let is_index_zero = k.index == 0;

            if has_coins || is_index_zero {
                if has_coins {
                    chunk_has_any = true;
                }
                addrs.push(AddressCoins { keys: k, xch, dig });
            }
        }

        if !chunk_has_any {
            // Entire chunk was empty — stop scanning.
            break;
        }
        start += CHUNK;
    }

    Ok(ScannedWallet { addrs })
}

// ===========================================================================
// Transaction history — wallet-side aggregation.
//
// There is NO SDK history API: chia-wallet-sdk constructs/signs spends but never
// stores or queries history. Sage records every coin add/remove during sync; we
// reconstruct the same view on demand from coinset by reading EVERY coin record
// (spent + unspent) at each of the wallet's HD puzzle hashes and mapping the coin
// lifecycle to transactions:
//
//   * a coin record at one of our puzzle hashes is an INCOMING tx (we received the
//     coin) at its `confirmed_block_index`;
//   * if that coin is `spent`, it is also an OUTGOING tx (we spent it) at its
//     `spent_block_index`.
//
// One coin can therefore yield up to two transactions (a receipt and a later
// spend). Results are sorted newest-first and paginated by [`TX_PAGE_SIZE`].
// ===========================================================================

/// Whether a transaction moved value INTO or OUT OF the wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxDirection {
    /// The wallet received a coin (a coin appeared at one of our puzzle hashes).
    In,
    /// The wallet spent a coin (a coin we held was removed).
    Out,
}

/// The asset a transaction moved. Classified from WHICH puzzle hash the coin
/// record was found at: the wallet's standard (XCH) puzzle hash → [`Xch`], the
/// wallet's CAT puzzle hash for a TAIL → [`Cat`]. `Nft`/`Did` are part of the
/// shape for callers that classify singleton coins, but puzzle-hash record
/// enumeration alone yields XCH and CAT.
///
/// [`Xch`]: TxAsset::Xch
/// [`Cat`]: TxAsset::Cat
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxAsset {
    /// Standard XCH.
    Xch,
    /// A CAT identified by its TAIL (`asset_id`).
    Cat { tail: Bytes32 },
    /// An NFT, identified by its launcher id.
    Nft { launcher_id: Bytes32 },
    /// A DID, identified by its launcher id.
    Did { launcher_id: Bytes32 },
}

/// Confirmation state of a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatus {
    /// In the mempool / not yet recorded in a block (block index 0).
    Pending,
    /// Recorded on-chain at a known block height.
    Confirmed,
}

/// One wallet transaction: a coin receipt or a coin spend, classified by asset and
/// direction. This is the wallet's own aggregation (no SDK equivalent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    /// In (received) or out (spent).
    pub direction: TxDirection,
    /// Which asset moved.
    pub asset: TxAsset,
    /// Amount in the asset's base units (mojos for XCH, base units for a CAT).
    pub amount: u64,
    /// Network fee in mojos attributable to this tx. Per-puzzle-hash records do
    /// not expose the full spend bundle, so the fee is not reconstructable here
    /// and is reported as 0; precise fees require correlating the whole bundle.
    pub fee: u64,
    /// Block height of the event (`confirmed_block_index` for a receipt,
    /// `spent_block_index` for a spend). 0 while pending.
    pub height: u32,
    /// Unix timestamp of the confirming block (0 if unknown / pending).
    pub timestamp: u64,
    /// The coin id(s) involved. One per event here (the coin received or spent).
    pub coin_ids: Vec<Bytes32>,
    /// Memos carried by the coin, if any (reserved; per-record enumeration does
    /// not surface memos, so this is currently empty).
    pub memos: Vec<Vec<u8>>,
    /// Pending (mempool) or confirmed (on-chain).
    pub status: TxStatus,
}

/// Classify a confirmation/spent block index into a [`TxStatus`].
fn status_for_height(height: u32) -> TxStatus {
    if height == 0 {
        TxStatus::Pending
    } else {
        TxStatus::Confirmed
    }
}

/// Build the transaction history for a scanned wallet, newest-first, paginated.
///
/// For every scanned HD address this reads ALL coin records (spent + unspent) at
/// the address's standard (XCH) puzzle hash and at its DIG-CAT puzzle hash, then
/// maps each coin's lifecycle to one or two [`Tx`] entries:
///
///   * an INCOMING tx for the coin's receipt (at `confirmed_block_index`), and
///   * an OUTGOING tx for its spend (at `spent_block_index`) if it is spent.
///
/// All entries are sorted newest-first (descending height, pending entries — height
/// 0 — sort to the front as the most recent activity), then the requested `page`
/// (0-based) of [`TX_PAGE_SIZE`] entries is returned. A `page` past the end yields
/// an empty vec.
///
/// Fee is reported as 0 (see [`Tx::fee`]); memos are empty (per-record enumeration
/// does not surface them). NFT/DID histories require singleton lineage walks and
/// are out of scope for this puzzle-hash aggregation.
pub async fn wallet_transactions(
    chain: &dyn ChainReads,
    scanned: &ScannedWallet,
    page: usize,
) -> Result<Vec<Tx>> {
    let mut txs: Vec<Tx> = Vec::new();

    for addr in &scanned.addrs {
        let owner_ph = addr.keys.owner_puzzle_hash;
        // XCH records at the standard puzzle hash.
        let xch_recs = chain.coin_records_by_puzzle_hash(owner_ph, true).await?;
        collect_txs(&xch_recs, TxAsset::Xch, &mut txs);

        // DIG-CAT records at the wallet's CAT puzzle hash for the DIG TAIL.
        let dig_recs = chain
            .coin_records_by_puzzle_hash(dig_cat_puzzle_hash(owner_ph), true)
            .await?;
        collect_txs(&dig_recs, TxAsset::Cat { tail: DIG_ASSET_ID }, &mut txs);
    }

    // Newest-first. Pending events (height 0) are the most recent activity, so
    // they sort ahead of confirmed ones; among confirmed, higher height is newer.
    // `Out` (spend) sorts ahead of `In` (receipt) at the same height so a coin's
    // spend (the later event) precedes its earlier receipt when both are pending.
    txs.sort_by(|a, b| {
        sort_key(a.height)
            .cmp(&sort_key(b.height))
            .then_with(|| dir_rank(a.direction).cmp(&dir_rank(b.direction)))
    });

    let start = page.saturating_mul(TX_PAGE_SIZE);
    Ok(txs.into_iter().skip(start).take(TX_PAGE_SIZE).collect())
}

/// Ascending sort key for newest-first ordering: pending (height 0) is the newest
/// activity and gets the smallest key (`0`); among confirmed events a HIGHER height
/// is newer and gets a smaller key (`u32::MAX - height`). Sorting ascending by this
/// key therefore yields pending first, then descending height.
fn sort_key(height: u32) -> u32 {
    if height == 0 {
        0
    } else {
        u32::MAX - height
    }
}

/// Tie-break rank so an outgoing (spend) event sorts before an incoming (receipt)
/// at the same height.
fn dir_rank(d: TxDirection) -> u8 {
    match d {
        TxDirection::Out => 0,
        TxDirection::In => 1,
    }
}

/// Map a slice of coin records (all of one asset) into transactions, pushing each
/// receipt and (if spent) spend into `out`.
fn collect_txs(records: &[crate::coinset::CoinRecord], asset: TxAsset, out: &mut Vec<Tx>) {
    for rec in records {
        let coin_id = rec.coin.coin_id();
        // Receipt (incoming).
        out.push(Tx {
            direction: TxDirection::In,
            asset,
            amount: rec.coin.amount,
            fee: 0,
            height: rec.confirmed_block_index,
            timestamp: rec.timestamp,
            coin_ids: vec![coin_id],
            memos: Vec::new(),
            status: status_for_height(rec.confirmed_block_index),
        });
        // Spend (outgoing) if the coin has been spent.
        if rec.spent {
            out.push(Tx {
                direction: TxDirection::Out,
                asset,
                amount: rec.coin.amount,
                fee: 0,
                height: rec.spent_block_index,
                timestamp: rec.timestamp,
                coin_ids: vec![coin_id],
                memos: Vec::new(),
                status: status_for_height(rec.spent_block_index),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::keys::derive_indexed_keys;
    use chia_protocol::{Bytes32, Coin};

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// Derive puzzle hashes for indices 0..n using the ABANDON mnemonic.
    fn ph(index: u32) -> Bytes32 {
        derive_indexed_keys(ABANDON, index..index + 1)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .owner_puzzle_hash
    }

    /// Derive the DIG CAT puzzle hash for a given index.
    fn dig_ph(index: u32) -> Bytes32 {
        dig_cat_puzzle_hash(ph(index))
    }

    #[tokio::test]
    async fn scan_aggregates_xch_and_dig_across_indices() {
        // Seed: XCH at index 0 (1_000_000) + index 2 (2_000_000);
        //       DIG CAT at index 1 (500_000).
        let mut mock = MockChain::default();

        // XCH at index 0
        let ph0 = ph(0);
        mock.coins_by_ph.insert(
            ph0,
            vec![Coin::new(Bytes32::from([1u8; 32]), ph0, 1_000_000)],
        );

        // XCH at index 2
        let ph2 = ph(2);
        mock.coins_by_ph.insert(
            ph2,
            vec![Coin::new(Bytes32::from([2u8; 32]), ph2, 2_000_000)],
        );

        // DIG CAT at index 1
        let dig1 = dig_ph(1);
        mock.coins_by_ph.insert(
            dig1,
            vec![Coin::new(Bytes32::from([3u8; 32]), dig1, 500_000)],
        );

        let w = scan_wallet(&mock, ABANDON).await.unwrap();

        assert_eq!(
            w.xch_balance(),
            3_000_000,
            "XCH should sum index 0 + index 2"
        );
        assert_eq!(w.dig_balance(), 500_000, "DIG should sum index 1");
    }

    #[tokio::test]
    async fn scan_wallet_coins_only_at_index_0() {
        // Only index 0 has coins — still scanned and returned.
        let mut mock = MockChain::default();
        let ph0 = ph(0);
        mock.coins_by_ph.insert(
            ph0,
            vec![Coin::new(Bytes32::from([10u8; 32]), ph0, 9_000_000)],
        );

        let w = scan_wallet(&mock, ABANDON).await.unwrap();

        assert_eq!(w.xch_balance(), 9_000_000);
        assert_eq!(w.dig_balance(), 0);
        assert_eq!(w.addrs.len(), 1, "only index 0 should be kept");
        assert_eq!(w.addrs[0].keys.index, 0);
    }

    #[tokio::test]
    async fn scan_empty_wallet_still_keeps_index_0() {
        // Completely empty wallet: index 0 must still be present.
        let mock = MockChain::default();
        let w = scan_wallet(&mock, ABANDON).await.unwrap();

        assert_eq!(w.xch_balance(), 0);
        assert_eq!(w.dig_balance(), 0);
        assert_eq!(w.addrs.len(), 1, "index 0 always kept");
        assert_eq!(w.addrs[0].keys.index, 0);
    }

    #[tokio::test]
    async fn signing_keys_covers_all_kept_addresses() {
        let mut mock = MockChain::default();

        // Seed index 0 and index 1 with XCH so both are kept.
        let ph0 = ph(0);
        let ph1 = ph(1);
        mock.coins_by_ph
            .insert(ph0, vec![Coin::new(Bytes32::from([20u8; 32]), ph0, 100)]);
        mock.coins_by_ph
            .insert(ph1, vec![Coin::new(Bytes32::from([21u8; 32]), ph1, 200)]);

        let w = scan_wallet(&mock, ABANDON).await.unwrap();

        // signing_keys must return one key per kept address.
        assert_eq!(w.signing_keys().len(), w.addrs.len());
    }

    // -------------------------------------------------------------------------
    // Transaction history (wallet_transactions).
    // -------------------------------------------------------------------------

    use crate::coinset::CoinRecord;

    /// Build a coin record at `puzzle_hash` with explicit spent state + heights.
    fn rec(
        parent: [u8; 32],
        puzzle_hash: Bytes32,
        amount: u64,
        confirmed_h: u32,
        spent: bool,
        spent_h: u32,
    ) -> CoinRecord {
        CoinRecord {
            coin: Coin::new(Bytes32::new(parent), puzzle_hash, amount),
            spent,
            confirmed_block_index: confirmed_h,
            spent_block_index: spent_h,
            timestamp: 1_700_000_000 + confirmed_h as u64,
            coinbase: false,
        }
    }

    /// A single-address (index 0) scanned wallet for history tests.
    fn scanned_index0() -> ScannedWallet {
        let keys = derive_indexed_keys(ABANDON, 0..1).unwrap();
        ScannedWallet {
            addrs: vec![AddressCoins {
                keys: keys.into_iter().next().unwrap(),
                xch: vec![],
                dig: vec![],
            }],
        }
    }

    // A spent incoming coin yields BOTH an incoming receipt and an outgoing spend;
    // an unspent incoming coin yields only a receipt. Directions + amounts correct.
    #[tokio::test]
    async fn wallet_transactions_maps_incoming_and_outgoing() {
        let mut mock = MockChain::default();
        let ph0 = ph(0);
        mock.records_by_ph.insert(
            ph0,
            vec![
                // received 1_000_000 at h=100, later spent at h=150 (in + out)
                rec([1u8; 32], ph0, 1_000_000, 100, true, 150),
                // received 2_000_000 at h=120, still unspent (in only)
                rec([2u8; 32], ph0, 2_000_000, 120, false, 0),
            ],
        );

        let scanned = scanned_index0();
        let txs = wallet_transactions(&mock, &scanned, 0).await.unwrap();

        // 2 receipts + 1 spend = 3 events.
        assert_eq!(txs.len(), 3);

        let ins: Vec<_> = txs
            .iter()
            .filter(|t| t.direction == TxDirection::In)
            .collect();
        let outs: Vec<_> = txs
            .iter()
            .filter(|t| t.direction == TxDirection::Out)
            .collect();
        assert_eq!(ins.len(), 2, "two receipts");
        assert_eq!(outs.len(), 1, "one spend");

        // The single outgoing is the spend of the 1_000_000 coin at height 150.
        assert_eq!(outs[0].amount, 1_000_000);
        assert_eq!(outs[0].height, 150);
        assert_eq!(outs[0].asset, TxAsset::Xch);
        assert_eq!(outs[0].status, TxStatus::Confirmed);

        // All events are XCH (records were at the standard puzzle hash).
        assert!(txs.iter().all(|t| t.asset == TxAsset::Xch));
    }

    // Newest-first ordering: highest height first; a pending event (height 0)
    // sorts ahead of all confirmed events.
    #[tokio::test]
    async fn wallet_transactions_sorted_newest_first_with_pending_on_top() {
        let mut mock = MockChain::default();
        let ph0 = ph(0);
        mock.records_by_ph.insert(
            ph0,
            vec![
                rec([1u8; 32], ph0, 10, 100, false, 0), // confirmed h=100
                rec([2u8; 32], ph0, 20, 300, false, 0), // confirmed h=300 (newer)
                rec([3u8; 32], ph0, 30, 0, false, 0),   // pending (h=0)
            ],
        );

        let scanned = scanned_index0();
        let txs = wallet_transactions(&mock, &scanned, 0).await.unwrap();
        assert_eq!(txs.len(), 3);

        // Order: pending (h=0) → h=300 → h=100.
        assert_eq!(txs[0].status, TxStatus::Pending);
        assert_eq!(txs[0].amount, 30);
        assert_eq!(txs[1].height, 300);
        assert_eq!(txs[2].height, 100);
    }

    // DIG-CAT records at the wallet's CAT puzzle hash classify as Cat{DIG tail}.
    #[tokio::test]
    async fn wallet_transactions_classifies_dig_cat() {
        let mut mock = MockChain::default();
        let dig0 = dig_ph(0);
        mock.records_by_ph
            .insert(dig0, vec![rec([7u8; 32], dig0, 500_000, 200, false, 0)]);

        let scanned = scanned_index0();
        let txs = wallet_transactions(&mock, &scanned, 0).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].asset, TxAsset::Cat { tail: DIG_ASSET_ID });
        assert_eq!(txs[0].amount, 500_000);
        assert_eq!(txs[0].direction, TxDirection::In);
    }

    // Pagination: TX_PAGE_SIZE per page, newest-first across pages, empty past end.
    #[tokio::test]
    async fn wallet_transactions_paginates() {
        let mut mock = MockChain::default();
        let ph0 = ph(0);
        // TX_PAGE_SIZE + 5 unspent receipts at increasing heights (1..=N).
        let n = TX_PAGE_SIZE + 5;
        let records: Vec<CoinRecord> = (1..=n)
            .map(|i| {
                let mut parent = [0u8; 32];
                parent[0] = (i & 0xff) as u8;
                parent[1] = (i >> 8) as u8;
                rec(parent, ph0, i as u64, i as u32, false, 0)
            })
            .collect();
        mock.records_by_ph.insert(ph0, records);

        let scanned = scanned_index0();

        let page0 = wallet_transactions(&mock, &scanned, 0).await.unwrap();
        assert_eq!(page0.len(), TX_PAGE_SIZE, "first page is full");
        // Newest-first: the very first entry is the highest height (n).
        assert_eq!(page0[0].height, n as u32);

        let page1 = wallet_transactions(&mock, &scanned, 1).await.unwrap();
        assert_eq!(page1.len(), 5, "second page holds the remaining 5");

        let page2 = wallet_transactions(&mock, &scanned, 2).await.unwrap();
        assert!(page2.is_empty(), "page past the end is empty");
    }
}
