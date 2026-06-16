//! DIG CAT coins over coinset: locate, value, and SPEND the wallet's DIG.
//!
//! Two responsibilities beyond balance reporting:
//!   1. `dig_cats` — reconstruct the wallet's unspent DIG CAT coins (with lineage
//!      proofs) over coinset, so they can be spent. This mirrors DataLayer-Driver's
//!      `DigCoin::from_coin` (which sources parent state from a P2P `Peer`); here we
//!      source the parent coin + its puzzle/solution from coinset reads instead.
//!   2. `build_dig_payment` — build the (UNSIGNED) CAT coin spends that send `amount`
//!      DIG to the treasury (memo = store id) and return the change to the owner.
//!      The anchor concatenates these with the singleton spends and signs the whole
//!      bundle with the owner's synthetic key (the CAT inner puzzle is the standard
//!      puzzle of that same synthetic key).
use crate::coinset::ChainReads;
use crate::dig::{treasury_inner_puzzle_hash, DIG_ASSET_ID};
use crate::error::{ChainError, Result};
use crate::keys::{IndexedKeys, WalletKeys};
use chia::puzzles::cat::CatArgs;
use chia_protocol::{Bytes32, CoinSpend};
use chia_wallet_sdk::driver::{Action, Cat, Id, Puzzle, Relation, SpendContext, Spends};
use chia_wallet_sdk::prelude::TreeHash;
use datalayer_driver::PublicKey;
use indexmap::{indexmap, IndexMap};

/// The coinset puzzle hash where `owner_puzzle_hash`'s DIG CAT coins live.
pub fn dig_cat_puzzle_hash(owner_puzzle_hash: Bytes32) -> Bytes32 {
    let ph = CatArgs::curry_tree_hash(DIG_ASSET_ID, TreeHash::from(owner_puzzle_hash)).to_bytes();
    Bytes32::from(ph)
}

/// Total spendable DIG (base units) at the wallet's DIG CAT puzzle hash.
pub async fn dig_balance(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<u64> {
    let coins = chain
        .unspent_coins(dig_cat_puzzle_hash(owner_puzzle_hash))
        .await?;
    Ok(coins.iter().map(|c| c.amount).sum())
}

/// Reconstruct the wallet's unspent DIG CAT coins (with lineage proofs) over
/// coinset, ready to be spent.
///
/// For each unspent coin at `dig_cat_puzzle_hash(owner_ph)`:
///   * read its own coin record to learn `confirmed_block_index` — the height at
///     which its PARENT was spent (a CAT child is created by spending its parent,
///     so the child's confirmation block IS the parent's spend block);
///   * fetch the parent spend (`coin_spend(child.parent_coin_info, child_height)`),
///     which carries the parent coin, its puzzle reveal, and its solution;
///   * run `Cat::parse_children(parent_coin, parent_puzzle, parent_solution)` and
///     select the child whose `coin_id` matches, whose `asset_id == DIG_ASSET_ID`,
///     and which carries a `lineage_proof` (required to spend a CAT).
///
/// This is `DigCoin::from_coin` with coinset reads swapped in for the Peer calls.
///
/// This is also available as `dig_cats_for` for multi-address callers that want
/// to call per address and concatenate.
pub async fn dig_cats(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<Vec<Cat>> {
    let cat_ph = dig_cat_puzzle_hash(owner_puzzle_hash);
    let coins = chain.unspent_coins(cat_ph).await?;

    let mut cats = Vec::with_capacity(coins.len());
    for coin in coins {
        let coin_id = coin.coin_id();

        // The child's own confirmation height == the height its parent was spent.
        let rec = chain
            .coin_record(coin_id)
            .await?
            .ok_or_else(|| ChainError::Chain(format!("DIG coin {coin_id:?} record not found")))?;
        let parent_spent_height = rec.confirmed_block_index;

        // Fetch the parent's spend (puzzle + solution) at that height.
        let parent_spend = chain
            .coin_spend(coin.parent_coin_info, parent_spent_height)
            .await?
            .ok_or_else(|| {
                ChainError::Chain(format!(
                    "DIG coin {coin_id:?}: parent spend {:?} not found at height {parent_spent_height}",
                    coin.parent_coin_info
                ))
            })?;

        // Parse the CAT children produced by spending the parent and pick ours.
        let mut ctx = SpendContext::new();
        let parent_puzzle_ptr = ctx
            .alloc(&parent_spend.puzzle_reveal)
            .map_err(|e| ChainError::Chain(format!("alloc parent puzzle: {e}")))?;
        let parent_puzzle = Puzzle::parse(&ctx, parent_puzzle_ptr);
        let parent_solution = ctx
            .alloc(&parent_spend.solution)
            .map_err(|e| ChainError::Chain(format!("alloc parent solution: {e}")))?;

        let children =
            Cat::parse_children(&mut ctx, parent_spend.coin, parent_puzzle, parent_solution)
                .map_err(|e| ChainError::Chain(format!("Cat::parse_children: {e}")))?
                .ok_or_else(|| {
                    ChainError::Chain(format!("DIG coin {coin_id:?}: parent is not a CAT"))
                })?;

        let cat = children
            .into_iter()
            .find(|c| {
                c.coin.coin_id() == coin_id
                    && c.info.asset_id == DIG_ASSET_ID
                    && c.lineage_proof.is_some()
            })
            .ok_or_else(|| {
                ChainError::Chain(format!(
                    "DIG coin {coin_id:?}: no matching DIG child with lineage proof"
                ))
            })?;

        cats.push(cat);
    }

    Ok(cats)
}

/// Greedily select DIG cats covering `amount`; returns the selected cats and the
/// sum of their amounts. Errors if the wallet's DIG is short.
fn select_dig_cats(dig_cats: &[Cat], amount: u64) -> Result<(Vec<Cat>, u64)> {
    let mut selected = Vec::new();
    let mut sum = 0u64;
    // Largest-first keeps the input count (and the spend size) small.
    let mut sorted: Vec<&Cat> = dig_cats.iter().collect();
    sorted.sort_by(|a, b| b.coin.amount.cmp(&a.coin.amount));
    for cat in sorted {
        if sum >= amount {
            break;
        }
        selected.push(*cat);
        sum += cat.coin.amount;
    }
    if sum < amount {
        return Err(ChainError::Chain(format!(
            "insufficient DIG: need {amount} have {sum}. Acquire DIG on TibetSwap: https://v2.tibetswap.io/"
        )));
    }
    Ok((selected, sum))
}

/// Build the (UNSIGNED) CAT coin spends that pay `amount` DIG to the treasury
/// (memo = `store_id`) and return the change to the owner.
///
/// The treasury output is a CAT `create_coin` to `treasury_inner_puzzle_hash()`
/// (the treasury's standard/inner puzzle hash; the action system wraps it in the
/// DIG CAT layer) carrying memos `[treasury_inner_ph (hint), store_id]`. The change
/// (`sum - amount`) is returned to the owner's inner puzzle hash with the owner hint
/// (the action system auto-creates the change coin at `Spends`'s change puzzle hash).
/// Each input CAT's inner spend is authorized by the owner's synthetic key via the
/// standard layer (supplied to `finish_with_keys`). No XCH is reserved (CAT value only).
///
/// Returns the CAT `CoinSpend`s UNSIGNED — the anchor signs the combined bundle.
pub fn build_dig_payment(
    keys: &WalletKeys,
    dig_cats: &[Cat],
    amount: u64,
    store_id: Bytes32,
) -> Result<Vec<CoinSpend>> {
    let (selected, sum) = select_dig_cats(dig_cats, amount)?;
    // Post-condition of selection: the chosen cats cover the requested amount.
    // (select_dig_cats already errors if short; this makes the guarantee explicit.)
    debug_assert!(sum >= amount, "selection must cover amount");

    let mut ctx = SpendContext::new();
    let treasury_ph = treasury_inner_puzzle_hash();

    // Memos for the treasury output: [treasury inner ph (hint), store id].
    let memos = ctx
        .memos(&[treasury_ph, store_id])
        .map_err(|e| ChainError::Chain(format!("alloc memos: {e}")))?;

    // Send `amount` DIG to the treasury CAT (inner ph + memos). The action system
    // auto-returns the change to `Spends`'s change puzzle hash (the owner inner ph,
    // hinted), so no explicit change action is required.
    let actions = [Action::send(
        Id::Existing(DIG_ASSET_ID),
        treasury_ph,
        amount,
        memos,
    )];

    // Change + input authorization both key off the owner's inner puzzle hash.
    let owner_ph = keys.owner_puzzle_hash;
    let mut spends = Spends::new(owner_ph);
    for cat in selected {
        spends.add(cat);
    }

    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply DIG send action: {e}")))?;

    let index_map = indexmap! { owner_ph => keys.synthetic_pk };
    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &index_map)
        .map_err(|e| ChainError::Chain(format!("finish DIG payment spends: {e}")))?;

    Ok(ctx.take())
}

/// Per-address alias for [`dig_cats`]: reconstruct DIG CAT coins for one owner
/// puzzle hash. Intended for multi-address callers that call once per DIG-bearing
/// address and concatenate the results.
#[inline]
pub async fn dig_cats_for(
    chain: &dyn ChainReads,
    owner_puzzle_hash: Bytes32,
) -> Result<Vec<Cat>> {
    dig_cats(chain, owner_puzzle_hash).await
}

/// Gather DIG cats across all scanned HD addresses and return them as a flat
/// `Vec<Cat>` ready for [`build_dig_payment_multi`].
///
/// Only addresses that actually hold DIG (`a.dig` non-empty) are queried for
/// lineage proofs, avoiding unnecessary coinset reads for empty addresses.
pub async fn dig_cats_multi(
    chain: &dyn ChainReads,
    w: &crate::wallet::ScannedWallet,
) -> Result<Vec<Cat>> {
    let mut all = Vec::new();
    for addr in &w.addrs {
        if !addr.dig.is_empty() {
            let cats = dig_cats_for(chain, addr.keys.owner_puzzle_hash).await?;
            all.extend(cats);
        }
    }
    Ok(all)
}

/// Multi-address variant of [`build_dig_payment`].
///
/// `dig_cats` may come from multiple HD addresses. `keys_by_ph` maps each
/// participating `owner_puzzle_hash` to its `synthetic_pk`, so the inner spend
/// for each CAT is authorized by the correct key. Change returns to `change_ph`
/// (index 0's `owner_puzzle_hash`).
///
/// The bundle is UNSIGNED — the anchor signs the combined bundle with all keys.
pub fn build_dig_payment_multi<'a>(
    keys_iter: impl Iterator<Item = &'a IndexedKeys>,
    change_ph: Bytes32,
    dig_cats: &[Cat],
    amount: u64,
    store_id: Bytes32,
) -> Result<Vec<CoinSpend>> {
    let (selected, sum) = select_dig_cats(dig_cats, amount)?;
    debug_assert!(sum >= amount, "selection must cover amount");

    let mut ctx = SpendContext::new();
    let treasury_ph = treasury_inner_puzzle_hash();

    let memos = ctx
        .memos(&[treasury_ph, store_id])
        .map_err(|e| ChainError::Chain(format!("alloc memos: {e}")))?;

    let actions = [Action::send(
        Id::Existing(DIG_ASSET_ID),
        treasury_ph,
        amount,
        memos,
    )];

    let mut spends = Spends::new(change_ph);
    for cat in selected {
        spends.add(cat);
    }

    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply DIG send action: {e}")))?;

    // Build the key map covering all participating addresses.
    let mut index_map: IndexMap<Bytes32, PublicKey> = IndexMap::new();
    for k in keys_iter {
        index_map.insert(k.owner_puzzle_hash, k.synthetic_pk);
    }
    // Ensure change_ph is always present (index 0).
    // The iterator already includes index 0, but guard against an empty iterator.
    if index_map.is_empty() {
        return Err(ChainError::Chain(
            "build_dig_payment_multi: keys_iter must not be empty".into(),
        ));
    }

    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &index_map)
        .map_err(|e| ChainError::Chain(format!("finish DIG payment spends: {e}")))?;

    Ok(ctx.take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::keys::derive_wallet_keys;
    use chia_protocol::Coin;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn dig_cat_puzzle_hash_is_32_bytes_and_stable() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let a = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        let b = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        assert_eq!(a, b);
        assert_eq!(a.to_bytes().len(), 32);
        // Print the value so it can be pinned as a golden in a later task.
        println!("ABANDON dig_cat_puzzle_hash = {}", hex::encode(a));
    }

    #[tokio::test]
    async fn dig_balance_sums_cat_coins() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let cat_ph = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        mock.coins_by_ph.insert(
            cat_ph,
            vec![
                Coin::new(Bytes32::default(), cat_ph, 60_000),
                Coin::new(Bytes32::from([1u8; 32]), cat_ph, 40_000),
            ],
        );
        assert_eq!(
            dig_balance(&mock, keys.owner_puzzle_hash).await.unwrap(),
            100_000
        );
    }

    // ----- select_dig_cats: the offline-verifiable selection/amount/error core -----
    //
    // A real `Cat` cannot be constructed offline without running CLVM (its
    // `info.p2_puzzle_hash` and `lineage_proof` come out of `Cat::parse_children`
    // over a real parent spend). So `dig_cats` reconstruction and the full
    // `build_dig_payment` spend are validated LIVE (like `sync_datastore`). The
    // selection/amount/error logic — which is the part with branching — is unit
    // tested here against synthetic `Cat`s built from `Cat::new` (no CLVM needed).

    fn dig_cat(amount: u64, parent: u8) -> Cat {
        use chia_wallet_sdk::driver::CatInfo;
        // A bare Cat with the right asset id + amount. lineage_proof is None here
        // (selection does not read it), so this is NOT spendable — selection-only.
        let coin = Coin::new(
            Bytes32::from([parent; 32]),
            Bytes32::from([0xcau8; 32]),
            amount,
        );
        Cat::new(
            coin,
            None,
            CatInfo::new(DIG_ASSET_ID, None, Bytes32::from([0x11u8; 32])),
        )
    }

    #[test]
    fn select_covers_amount_largest_first() {
        let cats = vec![dig_cat(40_000, 1), dig_cat(70_000, 2), dig_cat(5_000, 3)];
        let (sel, sum) = select_dig_cats(&cats, 100_000).unwrap();
        // 70_000 + 40_000 = 110_000 >= 100_000 (largest-first, stops early).
        assert_eq!(sum, 110_000);
        assert_eq!(sel.len(), 2);
        assert_eq!(sel[0].coin.amount, 70_000);
        assert_eq!(sel[1].coin.amount, 40_000);
    }

    #[test]
    fn select_exact_amount_single_coin() {
        let cats = vec![dig_cat(100_000, 1)];
        let (sel, sum) = select_dig_cats(&cats, 100_000).unwrap();
        assert_eq!(sum, 100_000);
        assert_eq!(sel.len(), 1);
    }

    #[test]
    fn select_errors_when_insufficient() {
        let cats = vec![dig_cat(40_000, 1), dig_cat(30_000, 2)];
        let err = select_dig_cats(&cats, 100_000).unwrap_err();
        match err {
            ChainError::Chain(msg) => {
                assert!(msg.contains("insufficient DIG"), "got: {msg}");
                assert!(msg.contains("need 100000"), "got: {msg}");
                assert!(msg.contains("have 70000"), "got: {msg}");
            }
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    #[test]
    fn shortfall_error_contains_tibetswap_link() {
        let cats = vec![dig_cat(10_000, 1)];
        let err = select_dig_cats(&cats, 50_000).unwrap_err();
        match err {
            ChainError::Chain(msg) => {
                assert!(
                    msg.contains("tibetswap") || msg.contains("TibetSwap"),
                    "shortfall error must mention TibetSwap, got: {msg}"
                );
                assert!(
                    msg.contains("https://v2.tibetswap.io/"),
                    "shortfall error must include the TibetSwap URL, got: {msg}"
                );
            }
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    #[test]
    fn select_errors_on_empty() {
        let err = select_dig_cats(&[], 10_000).unwrap_err();
        assert!(matches!(err, ChainError::Chain(_)));
    }

    #[test]
    fn build_dig_payment_errors_when_insufficient() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let cats = vec![dig_cat(5_000, 1)];
        let err = build_dig_payment(&keys, &cats, 100_000, Bytes32::default()).unwrap_err();
        match err {
            ChainError::Chain(msg) => assert!(msg.contains("insufficient DIG"), "got: {msg}"),
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    // Offline error-path coverage for dig_cats reconstruction: an unspent DIG CAT
    // coin exists at the wallet's DIG CAT puzzle hash, but its OWN coin record is
    // missing (so the parent's spend height is unknown). Reconstruction must error
    // cleanly ("record not found") rather than panic. This exercises the first
    // fallible branch in dig_cats without needing real CLVM lineage. The happy
    // path (a parent that parses into DIG children) needs real chain data and is
    // covered by the ignored live test + the controller.
    #[tokio::test]
    async fn dig_cats_errors_when_parent_record_missing() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let owner_ph = keys.owner_puzzle_hash;
        let cat_ph = dig_cat_puzzle_hash(owner_ph);

        let mut mock = MockChain::default();
        // An unspent coin exists at the DIG CAT ph, so unspent_coins returns it...
        mock.coins_by_ph.insert(
            cat_ph,
            vec![Coin::new(Bytes32::from([3u8; 32]), cat_ph, 50_000)],
        );
        // ...but we deliberately seed NO `records` entry for that coin, so the
        // coin_record lookup returns None → "record not found".

        let res = dig_cats(&mock, owner_ph).await;
        assert!(res.is_err(), "missing parent record must error, not panic");
        match res.unwrap_err() {
            ChainError::Chain(msg) => {
                assert!(msg.contains("record not found"), "got: {msg}");
            }
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    // dig_cats over coinset and the full CAT spend (sending to the treasury with
    // a store-id memo + change) require real CLVM/lineage data and are validated
    // by the controller's live init/commit, plus the ignored live test below.
    #[tokio::test]
    #[ignore]
    async fn dig_cats_live_reconstruct() {
        use crate::coinset::Coinset;
        let chain = Coinset::mainnet();
        let phrase = std::fs::read_to_string("../../.testcredentials").unwrap();
        let keys = derive_wallet_keys(phrase.trim()).unwrap();
        let cats = dig_cats(&chain, keys.owner_puzzle_hash).await.unwrap();
        let total: u64 = cats.iter().map(|c| c.coin.amount).sum();
        println!(
            "reconstructed {} DIG cats, total {total} base units",
            cats.len()
        );
        for c in &cats {
            assert_eq!(c.info.asset_id, DIG_ASSET_ID);
            assert!(c.lineage_proof.is_some());
        }
        // Build a payment and confirm it produces spends without panicking.
        if total >= 1_000 {
            let pay = build_dig_payment(&keys, &cats, 1_000, Bytes32::from([7u8; 32])).unwrap();
            assert!(!pay.is_empty());
            println!("built {} DIG coin spends (UNSIGNED)", pay.len());
        }
    }
}
