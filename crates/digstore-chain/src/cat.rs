//! CAT coins over coinset: locate, value, and SPEND the wallet's tokens.
//!
//! The crate's CAT logic is **generic over the TAIL** (`asset_id`): the same
//! reconstruction / balance / send path works for ANY CAT, with DIG as a thin
//! specialization (`asset_id == DIG_ASSET_ID`). This is the wallet's Sage-parity
//! generic-CAT surface — `chip0002_getAssetBalance`/`getAssetCoins`/`chia_send`
//! for an arbitrary token, not just DIG.
//!
//! Responsibilities beyond balance reporting:
//!   1. [`reconstruct_cat_coins`] — reconstruct the wallet's unspent CAT coins (with
//!      lineage proofs) over coinset for a given `asset_id`, so they can be spent.
//!      This mirrors DataLayer-Driver's `DigCoin::from_coin` (which sources parent
//!      state from a P2P `Peer`); here we source the parent coin + its
//!      puzzle/solution from coinset reads instead.
//!   2. [`build_cat_send`] — build AND sign the CAT coin spends that send `amount`
//!      of `asset_id` to a recipient (with optional memos) and return the change to
//!      the owner. Mirrors [`crate::send::build_xch_send`]: **pure build + sign**,
//!      NEVER broadcasts (the dig-wallet `DIG_WALLET_ALLOW_BROADCAST` gate is the
//!      policy layer above this).
//!   3. [`build_dig_payment`] — the DIG-specific store-payment path (memo = store id,
//!      recipient = treasury), kept as a specialization the anchor concatenates with
//!      the singleton spends and signs as one bundle.
use crate::coinset::ChainReads;
use crate::dig::{treasury_inner_puzzle_hash, DIG_ASSET_ID};
use crate::error::{ChainError, Result};
use crate::keys::{IndexedKeys, WalletKeys};
use chia::puzzles::cat::CatArgs;
use chia_protocol::{Bytes32, CoinSpend, SpendBundle};
use chia_wallet_sdk::driver::{Action, Cat, Id, Puzzle, Relation, SpendContext, Spends};
use chia_wallet_sdk::prelude::TreeHash;
use datalayer_driver::{sign_coin_spends, PublicKey};
use indexmap::{indexmap, IndexMap};

/// The coinset puzzle hash where `owner_puzzle_hash`'s coins for `asset_id` live.
///
/// Generic over the TAIL: a CAT coin lives at the outer puzzle hash that curries
/// the asset id (TAIL hash) around the owner's inner (standard) puzzle hash. DIG is
/// the special case `asset_id == DIG_ASSET_ID` ([`dig_cat_puzzle_hash`]).
pub fn cat_puzzle_hash(owner_puzzle_hash: Bytes32, asset_id: Bytes32) -> Bytes32 {
    let ph = CatArgs::curry_tree_hash(asset_id, TreeHash::from(owner_puzzle_hash)).to_bytes();
    Bytes32::from(ph)
}

/// The coinset puzzle hash where `owner_puzzle_hash`'s DIG CAT coins live.
/// Thin specialization of [`cat_puzzle_hash`] with `asset_id == DIG_ASSET_ID`.
pub fn dig_cat_puzzle_hash(owner_puzzle_hash: Bytes32) -> Bytes32 {
    cat_puzzle_hash(owner_puzzle_hash, DIG_ASSET_ID)
}

/// Total spendable base units of `asset_id` at the wallet's CAT puzzle hash for that
/// asset. Generic over the TAIL; DIG is [`dig_balance`].
pub async fn cat_balance(
    chain: &dyn ChainReads,
    owner_puzzle_hash: Bytes32,
    asset_id: Bytes32,
) -> Result<u64> {
    let coins = chain
        .unspent_coins(cat_puzzle_hash(owner_puzzle_hash, asset_id))
        .await?;
    Ok(coins.iter().map(|c| c.amount).sum())
}

/// Total spendable DIG (base units) at the wallet's DIG CAT puzzle hash.
/// Thin specialization of [`cat_balance`] with `asset_id == DIG_ASSET_ID`.
pub async fn dig_balance(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<u64> {
    cat_balance(chain, owner_puzzle_hash, DIG_ASSET_ID).await
}

/// Reconstruct the wallet's unspent CAT coins (with lineage proofs) over coinset
/// for an arbitrary `asset_id` (TAIL), ready to be spent. Generic over the TAIL;
/// DIG is [`dig_cats`].
///
/// For each unspent coin at `cat_puzzle_hash(owner_ph, asset_id)`:
///   * read its own coin record to learn `confirmed_block_index` — the height at
///     which its PARENT was spent (a CAT child is created by spending its parent,
///     so the child's confirmation block IS the parent's spend block);
///   * fetch the parent spend (`coin_spend(child.parent_coin_info, child_height)`),
///     which carries the parent coin, its puzzle reveal, and its solution;
///   * run `Cat::parse_children(parent_coin, parent_puzzle, parent_solution)` and
///     select the child whose `coin_id` matches, whose `asset_id` matches, and which
///     carries a `lineage_proof` (required to spend a CAT).
///
/// This is `DigCoin::from_coin` with coinset reads swapped in for the Peer calls,
/// generalized to any TAIL.
pub async fn reconstruct_cat_coins(
    chain: &dyn ChainReads,
    owner_puzzle_hash: Bytes32,
    asset_id: Bytes32,
) -> Result<Vec<Cat>> {
    let cat_ph = cat_puzzle_hash(owner_puzzle_hash, asset_id);
    let coins = chain.unspent_coins(cat_ph).await?;

    let mut cats = Vec::with_capacity(coins.len());
    for coin in coins {
        let coin_id = coin.coin_id();

        // The child's own confirmation height == the height its parent was spent.
        let rec = chain
            .coin_record(coin_id)
            .await?
            .ok_or_else(|| ChainError::Chain(format!("CAT coin {coin_id:?} record not found")))?;
        let parent_spent_height = rec.confirmed_block_index;

        // Fetch the parent's spend (puzzle + solution) at that height.
        let parent_spend = chain
            .coin_spend(coin.parent_coin_info, parent_spent_height)
            .await?
            .ok_or_else(|| {
                ChainError::Chain(format!(
                    "CAT coin {coin_id:?}: parent spend {:?} not found at height {parent_spent_height}",
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
                    ChainError::Chain(format!("CAT coin {coin_id:?}: parent is not a CAT"))
                })?;

        let cat = children
            .into_iter()
            .find(|c| {
                c.coin.coin_id() == coin_id
                    && c.info.asset_id == asset_id
                    && c.lineage_proof.is_some()
            })
            .ok_or_else(|| {
                ChainError::Chain(format!(
                    "CAT coin {coin_id:?}: no matching child with lineage proof for asset {asset_id:?}"
                ))
            })?;

        cats.push(cat);
    }

    Ok(cats)
}

/// Reconstruct the wallet's unspent DIG CAT coins (with lineage proofs) over coinset.
/// Thin specialization of [`reconstruct_cat_coins`] with `asset_id == DIG_ASSET_ID`.
///
/// This is also available as `dig_cats_for` for multi-address callers that want
/// to call per address and concatenate.
pub async fn dig_cats(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<Vec<Cat>> {
    reconstruct_cat_coins(chain, owner_puzzle_hash, DIG_ASSET_ID).await
}

/// Greedily select CAT coins covering `amount` (largest-first); returns the selected
/// cats and the sum of their amounts. Errors if the wallet is short, using `shortfall`
/// to render the (asset-specific) message.
fn select_cats(
    cats: &[Cat],
    amount: u64,
    shortfall: impl Fn(u64, u64) -> String,
) -> Result<(Vec<Cat>, u64)> {
    let mut selected = Vec::new();
    let mut sum = 0u64;
    // Largest-first keeps the input count (and the spend size) small.
    let mut sorted: Vec<&Cat> = cats.iter().collect();
    sorted.sort_by(|a, b| b.coin.amount.cmp(&a.coin.amount));
    for cat in sorted {
        if sum >= amount {
            break;
        }
        selected.push(*cat);
        sum += cat.coin.amount;
    }
    if sum < amount {
        return Err(ChainError::Chain(shortfall(amount, sum)));
    }
    Ok((selected, sum))
}

/// Greedily select DIG cats covering `amount`; returns the selected cats and the
/// sum of their amounts. Errors (with the TibetSwap shortfall hint) if DIG is short.
fn select_dig_cats(dig_cats: &[Cat], amount: u64) -> Result<(Vec<Cat>, u64)> {
    select_cats(dig_cats, amount, |need, have| {
        format!(
            "insufficient DIG: need {need} have {have}. Acquire DIG on TibetSwap: https://v2.tibetswap.io/"
        )
    })
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
    // Single-key (index-0) variant: change + every input authorization key off the
    // owner's inner puzzle hash with its one synthetic key.
    build_dig_payment_inner(
        keys.owner_puzzle_hash,
        indexmap! { keys.owner_puzzle_hash => keys.synthetic_pk },
        dig_cats,
        amount,
        store_id,
    )
}

/// The single place the DIG treasury payment spend is constructed (the
/// [`build_dig_payment`] / [`build_dig_payment_multi`] / [`build_dig_store_payment`]
/// common core). Selects DIG cats covering `amount`, sends them to the treasury inner
/// puzzle hash with memos `[treasury_inner_ph (hint), store_id]`, and returns the change
/// to `change_ph` (hinted). `keys_by_ph` maps each participating address's owner inner
/// puzzle hash to its synthetic key so the per-input inner spends are authorized
/// correctly (one entry for single-key, many for the HD ring). Returns UNSIGNED spends.
fn build_dig_payment_inner(
    change_ph: Bytes32,
    keys_by_ph: IndexMap<Bytes32, PublicKey>,
    dig_cats: &[Cat],
    amount: u64,
    store_id: Bytes32,
) -> Result<Vec<CoinSpend>> {
    let (selected, sum) = select_dig_cats(dig_cats, amount)?;
    // Post-condition of selection: the chosen cats cover the requested amount.
    // (select_dig_cats already errors if short; this makes the guarantee explicit.)
    debug_assert!(sum >= amount, "selection must cover amount");
    if keys_by_ph.is_empty() {
        return Err(ChainError::Chain(
            "build_dig_payment: keys_by_ph must not be empty".into(),
        ));
    }

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

    let mut spends = Spends::new(change_ph);
    for cat in selected {
        spends.add(cat);
    }

    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply DIG send action: {e}")))?;

    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &keys_by_ph)
        .map_err(|e| ChainError::Chain(format!("finish DIG payment spends: {e}")))?;

    Ok(ctx.take())
}

/// Build the (UNSIGNED) DIG-CAT coin spends that pay `amount` base units of $DIG to the
/// DIG treasury for a capsule (commit) — the chip35-canonical store-payment shape.
///
/// This is the byte-mirror of chip35 0.9.0's `build_dig_store_payment`
/// (`chip35_dl_coin::dig::build_dig_store_payment`): same parameter shape
/// (`buyer_synthetic_key`, `dig_cats`, `store_id`, `amount`), same treasury recipient
/// ([`treasury_inner_puzzle_hash`]), same memo layout `[treasury_inner_ph (hint),
/// store_id]`, same DIG asset id. The buyer's inner (owner) puzzle hash — the change
/// destination and the inner-spend authorizer — is derived from `buyer_synthetic_key`
/// (the standard puzzle of that synthetic key), exactly as chip35 does.
///
/// **MINT does NOT call this — minting a store is free of $DIG (#111).** Only a COMMIT
/// (root-advance = a capsule) concatenates these coin spends with the singleton update
/// into one atomic, co-signed bundle. Returns the CAT `CoinSpend`s UNSIGNED — the anchor
/// signs the combined bundle.
///
/// Single-key path. The CLI / wallet anchor uses the multi-address HD ring
/// [`build_dig_payment_multi`] (DIG gathered across HD addresses) for the same reason
/// the hub keeps its ring; this single-key entry point exists for the chip35-canonical
/// API shape and single-key callers.
pub fn build_dig_store_payment(
    buyer_synthetic_key: PublicKey,
    dig_cats: Vec<Cat>,
    store_id: Bytes32,
    amount: u64,
) -> Result<Vec<CoinSpend>> {
    if dig_cats.is_empty() {
        return Err(ChainError::Chain("dig_cats is empty".to_string()));
    }
    if dig_cats.iter().any(|c| c.info.asset_id != DIG_ASSET_ID) {
        return Err(ChainError::Chain(
            "dig_cats are not the DIG asset".to_string(),
        ));
    }
    // Derive the buyer's inner (owner) puzzle hash from the synthetic key — the change
    // destination + inner-spend authorizer (byte-mirror of chip35's StandardArgs curry).
    let owner_puzzle_hash: Bytes32 =
        chia::puzzles::standard::StandardArgs::curry_tree_hash(buyer_synthetic_key).into();
    build_dig_payment_inner(
        owner_puzzle_hash,
        indexmap! { owner_puzzle_hash => buyer_synthetic_key },
        &dig_cats,
        amount,
        store_id,
    )
}

/// The result of building a generic CAT send: the signed bundle plus the value plan.
/// `inputs == amount + change` always holds (no XCH fee is reserved from the CAT
/// value — a fee, if any, is paid in XCH by a co-spent standard coin, which the
/// action system reserves via [`Action::fee`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatSendPlan {
    /// Total base units of the selected input CAT coins.
    pub inputs: u64,
    /// Base units paid to the recipient.
    pub amount: u64,
    /// Base units returned to the owner (change), 0 if exact.
    pub change: u64,
    /// XCH mojos reserved as the network fee.
    pub fee: u64,
}

/// Build AND sign a generic CAT send: pay `amount` base units of `asset_id` to
/// `recipient_ph` (hinted, with optional `memos`) and return the change to the
/// owner, optionally reserving an XCH `fee`. Generic over the TAIL — the wallet's
/// Sage-parity `chia_send` for an arbitrary token.
///
/// `cats` are the owner's reconstructed CAT coins for `asset_id` (lineage proofs
/// required to spend), all held at `keys.owner_puzzle_hash`. The recipient output
/// carries `[recipient_ph (hint), ..memos]` so the receiving wallet can discover the
/// coin by hint (Sage's CAT receive); change is auto-created at the owner's hinted
/// inner puzzle hash by the action system. The CAT inner spends are authorized by
/// `keys.synthetic_pk` via the standard layer.
///
/// The `fee` is reserved as an XCH network fee inside the same spend (the action
/// system attaches it to the CAT spend's announcement ring), so the caller does not
/// pass a separate XCH coin here. Signs with the owner's synthetic key (AugScheme);
/// `for_testnet` selects the `agg_sig` network (mainnet in production). **Pure: does
/// NOT broadcast** — pushing is the caller's gated decision, exactly like
/// [`crate::send::build_xch_send`].
#[allow(clippy::too_many_arguments)]
pub fn build_cat_send(
    keys: &WalletKeys,
    cats: &[Cat],
    asset_id: Bytes32,
    recipient_ph: Bytes32,
    amount: u64,
    memos: &[Bytes32],
    fee: u64,
    for_testnet: bool,
) -> Result<(SpendBundle, CatSendPlan)> {
    if amount == 0 {
        return Err(ChainError::Chain(
            "send amount must be greater than zero".into(),
        ));
    }
    let (selected, sum) = select_cats(cats, amount, |need, have| {
        format!("insufficient CAT (asset {asset_id:?}): need {need} have {have}")
    })?;
    debug_assert!(sum >= amount, "selection must cover amount");

    let mut ctx = SpendContext::new();

    // The recipient output is hinted so the receiving wallet finds it by hint; any
    // caller-supplied memos follow the hint (Sage's CAT send-with-memo shape).
    let mut memo_items: Vec<Bytes32> = Vec::with_capacity(1 + memos.len());
    memo_items.push(recipient_ph);
    memo_items.extend_from_slice(memos);
    let memos = ctx
        .memos(&memo_items)
        .map_err(|e| ChainError::Chain(format!("alloc memos: {e}")))?;

    // Send `amount` of the asset to the recipient; the action system auto-returns the
    // change to `Spends`'s change puzzle hash (the owner inner ph, hinted). A non-zero
    // fee is reserved as an XCH network fee within the same spend.
    let mut actions = vec![Action::send(
        Id::Existing(asset_id),
        recipient_ph,
        amount,
        memos,
    )];
    if fee > 0 {
        actions.push(Action::fee(fee));
    }

    let owner_ph = keys.owner_puzzle_hash;
    let mut spends = Spends::new(owner_ph);
    for cat in selected {
        spends.add(cat);
    }

    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply CAT send action: {e}")))?;

    let index_map = indexmap! { owner_ph => keys.synthetic_pk };
    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &index_map)
        .map_err(|e| ChainError::Chain(format!("finish CAT send spends: {e}")))?;

    let coin_spends = ctx.take();
    // Sign with the owner's synthetic key. `for_testnet` selects the agg_sig network
    // (mainnet in production; the simulator validates against TESTNET11).
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        for_testnet,
    )
    .map_err(|e| ChainError::Chain(format!("sign CAT send: {e}")))?;
    let bundle = SpendBundle::new(coin_spends, signature);

    let plan = CatSendPlan {
        inputs: sum,
        amount,
        change: sum - amount,
        fee,
    };
    Ok((bundle, plan))
}

/// Per-address alias for [`dig_cats`]: reconstruct DIG CAT coins for one owner
/// puzzle hash. Intended for multi-address callers that call once per DIG-bearing
/// address and concatenate the results.
#[inline]
pub async fn dig_cats_for(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<Vec<Cat>> {
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
    // Build the key map covering all participating HD addresses (the DIG ring), then
    // delegate to the shared core. change_ph (index 0) must be among them.
    let mut keys_by_ph: IndexMap<Bytes32, PublicKey> = IndexMap::new();
    for k in keys_iter {
        keys_by_ph.insert(k.owner_puzzle_hash, k.synthetic_pk);
    }
    build_dig_payment_inner(change_ph, keys_by_ph, dig_cats, amount, store_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::keys::derive_wallet_keys;
    use chia_protocol::Coin;
    use chia_wallet_sdk::types::{run_puzzle, Condition};
    use clvm_traits::{FromClvm, ToClvm};
    use clvmr::Allocator;

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

    // ----- generic CAT: pure helpers (offline) -----

    #[test]
    fn cat_puzzle_hash_matches_dig_specialization() {
        // The generic puzzle-hash function with DIG_ASSET_ID must equal the DIG
        // specialization — DIG is just a particular TAIL.
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let generic = cat_puzzle_hash(keys.owner_puzzle_hash, DIG_ASSET_ID);
        assert_eq!(generic, dig_cat_puzzle_hash(keys.owner_puzzle_hash));
        // A different TAIL yields a different puzzle hash (the asset id is curried in).
        let other = cat_puzzle_hash(keys.owner_puzzle_hash, Bytes32::from([0x42u8; 32]));
        assert_ne!(other, generic);
        assert_eq!(other.to_bytes().len(), 32);
    }

    #[test]
    fn build_cat_send_rejects_zero_amount_and_shortfall() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let asset = Bytes32::from([0x42u8; 32]);
        // Zero amount is rejected up front.
        let err =
            build_cat_send(&keys, &[], asset, Bytes32::default(), 0, &[], 0, false).unwrap_err();
        assert!(matches!(&err, ChainError::Chain(m) if m.contains("greater than zero")));
        // A shortfall surfaces a clear, asset-tagged "insufficient CAT" message (not
        // the DIG/TibetSwap wording, which is specific to the DIG path).
        let err = build_cat_send(&keys, &[], asset, Bytes32::default(), 1_000, &[], 0, false)
            .unwrap_err();
        match err {
            ChainError::Chain(msg) => {
                assert!(msg.contains("insufficient CAT"), "got: {msg}");
                assert!(
                    !msg.contains("TibetSwap"),
                    "generic CAT must not mention DIG/TibetSwap"
                );
            }
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    // ----- generic CAT: full mint→send→balances round-trip on the Simulator -----

    /// Issue `amount` of a fresh CAT to `owner_ph` in the simulator, returning the
    /// spendable [`Cat`] (with lineage proof) and its asset id. Mirrors the offer
    /// module's helper; the inner puzzle is the standard puzzle of `owner_pk`.
    fn issue_cat_to(
        sim: &mut chia_sdk_test::Simulator,
        ctx: &mut SpendContext,
        owner_ph: Bytes32,
        owner_pk: PublicKey,
        owner_sk: &datalayer_driver::SecretKey,
        amount: u64,
    ) -> anyhow::Result<(Cat, Bytes32)> {
        use chia_wallet_sdk::driver::StandardLayer;
        use chia_wallet_sdk::types::Conditions;
        let xch = sim.new_coin(owner_ph, amount);
        let p2 = StandardLayer::new(owner_pk);
        let hint = ctx.hint(owner_ph)?;
        let (issue_cat, cats) = Cat::issue_with_coin(
            ctx,
            xch.coin_id(),
            amount,
            Conditions::new().create_coin(owner_ph, amount, hint),
        )?;
        p2.spend(ctx, xch, issue_cat)?;
        let asset_id = cats[0].info.asset_id;
        sim.spend_coins(ctx.take(), std::slice::from_ref(owner_sk))?;
        Ok((cats[0], asset_id))
    }

    #[test]
    fn cat_send_pays_recipient_with_change_and_validates_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        let mut sim = Simulator::new();
        let mut ctx = SpendContext::new();

        // The wallet (index 0). build_cat_send signs with synthetic_sk; the issued
        // CAT's inner puzzle is the standard puzzle of synthetic_pk, so the keys must
        // line up. (keys.rs guarantees index 0 == derive_wallet_keys.)
        let keys = derive_wallet_keys(ABANDON)?;
        let minted: u64 = 100_000;
        let (cat, asset_id) = issue_cat_to(
            &mut sim,
            &mut ctx,
            keys.owner_puzzle_hash,
            keys.synthetic_pk,
            &keys.synthetic_sk,
            minted,
        )?;

        // Send 30_000 of the minted CAT to a fresh recipient, with a memo, no fee.
        let recipient_ph = Bytes32::from([0xAAu8; 32]);
        let memo = Bytes32::from([0x77u8; 32]);
        let send_amount: u64 = 30_000;
        let (bundle, plan) = build_cat_send(
            &keys,
            &[cat],
            asset_id,
            recipient_ph,
            send_amount,
            &[memo],
            0,
            true, // simulator validates against TESTNET11 agg_sig data
        )?;
        assert_eq!(plan.inputs, minted);
        assert_eq!(plan.amount, send_amount);
        assert_eq!(plan.change, minted - send_amount);

        // Structural check: running the CAT OUTER puzzle morphs each inner CREATE_COIN
        // into a create to the CAT-WRAPPED puzzle hash, so the recipient output's
        // puzzle hash equals `cat_puzzle_hash(recipient_ph, asset_id)` with the exact
        // send amount. This proves the asset routes to the recipient's CAT address.
        let recipient_cat_ph = cat_puzzle_hash(recipient_ph, asset_id);
        let mut a = Allocator::new();
        let pays_recipient = bundle.coin_spends.iter().any(|cs| {
            let Ok(puzzle) = cs.puzzle_reveal.to_clvm(&mut a) else {
                return false;
            };
            let Ok(solution) = cs.solution.to_clvm(&mut a) else {
                return false;
            };
            let Ok(output) = run_puzzle(&mut a, puzzle, solution) else {
                return false;
            };
            let Ok(conds) = Vec::<Condition>::from_clvm(&a, output) else {
                return false;
            };
            conds.iter().any(|c| {
                matches!(c, Condition::CreateCoin(cc)
                    if cc.amount == send_amount && cc.puzzle_hash == recipient_cat_ph)
            })
        });
        assert!(
            pays_recipient,
            "a CAT spend must CREATE_COIN(cat_puzzle_hash(recipient), {send_amount})"
        );

        // The bundle must be a single valid transaction on the simulator (signature
        // verifies, ring balances). This proves build_cat_send produced a real,
        // broadcast-ready CAT spend for an arbitrary TAIL.
        sim.new_transaction(bundle)?;

        // After confirmation: recipient holds `send_amount`, owner holds the change,
        // queried through the GENERIC balance path (cat_balance) over the simulator's
        // coin set, proving balances/coins are correct for an arbitrary asset.
        let sim_chain = SimChain(&sim);
        let recipient_bal = cat_balance(&sim_chain, recipient_ph, asset_id).await_blocking()?;
        let owner_bal =
            cat_balance(&sim_chain, keys.owner_puzzle_hash, asset_id).await_blocking()?;
        assert_eq!(recipient_bal, send_amount, "recipient balance after send");
        assert_eq!(owner_bal, minted - send_amount, "owner change after send");
        Ok(())
    }

    /// A tiny [`ChainReads`] adapter over the simulator's coin store so the generic
    /// balance/coins path can be exercised post-send. Only `unspent_coins` is needed
    /// for `cat_balance`; the rest are unimplemented (this test never calls them).
    struct SimChain<'a>(&'a chia_sdk_test::Simulator);

    #[async_trait::async_trait]
    impl crate::coinset::ChainReads for SimChain<'_> {
        async fn unspent_coins(&self, ph: Bytes32) -> Result<Vec<Coin>> {
            // include_hints=false: query by exact puzzle hash (the CAT outer ph),
            // matching the real coinset `get_coin_records_by_puzzle_hashes` semantics.
            Ok(self.0.unspent_coins(ph, false))
        }
        async fn unspent_coins_by_hint(&self, _hint: Bytes32) -> Result<Vec<Coin>> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn coin_records_by_puzzle_hash(
            &self,
            _ph: Bytes32,
            _include_spent: bool,
        ) -> Result<Vec<crate::coinset::CoinRecord>> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn coin_record(&self, _name: Bytes32) -> Result<Option<crate::coinset::CoinInfo>> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn coin_spend(
            &self,
            _id: Bytes32,
            _h: u32,
        ) -> Result<Option<chia_protocol::CoinSpend>> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn peak_height(&self) -> Result<u32> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn push(&self, _bundle: SpendBundle) -> Result<()> {
            unimplemented!("SimChain only supports unspent_coins")
        }
        async fn estimate_fee(&self, _bundle: &SpendBundle, _target_secs: u64) -> Result<u64> {
            Ok(0)
        }
    }

    /// Block on a future inside a sync test (the round-trip test is sync so it can
    /// drive the simulator, which is not `Send`). A current-thread runtime suffices.
    trait BlockOn: std::future::Future + Sized {
        fn await_blocking(self) -> Self::Output {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("build current-thread runtime")
                .block_on(self)
        }
    }
    impl<F: std::future::Future> BlockOn for F {}
}
