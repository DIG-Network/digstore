//! Build + sign Chia datastore singleton spends (mint + update). Pure: callers
//! fetch unspent coins via `ChainReads` and broadcast the returned bundle via
//! `ChainReads::push`. Verified on mainnet in the Phase-0 prototype.

use crate::coinset::ChainReads;
use crate::error::{ChainError, Result};
use crate::keys::WalletKeys;
use chia_wallet_sdk::driver::{DriverError, Launcher, SpendContext, StandardLayer};
use chia_wallet_sdk::types::{conditions::CreateCoin, Condition, Conditions};
use datalayer_driver::{
    add_fee, select_coins, sign_coin_spends, update_store_metadata, Bytes32, Coin, CoinSpend,
    DataStore, DataStoreInnerSpend, DataStoreMetadata, DelegatedPuzzle, PublicKey, SpendBundle,
    SuccessResponse,
};

/// An XCH coin tagged with the wallet address it belongs to, so the spend can
/// be built with the correct `synthetic_pk` (each address has its own key).
#[derive(Clone, Debug)]
pub struct CoinWithKey {
    pub coin: Coin,
    pub synthetic_pk: PublicKey,
    pub owner_puzzle_hash: Bytes32,
}
use hex_literal::hex;

/// `sha256("datastore")` — the global launcher hint (kept as a second memo for
/// compatibility with DATASTORE_LAUNCHER_HINT-based tooling). Matches chip35 + datalayer.
const DATASTORE_LAUNCHER_HINT: Bytes32 = Bytes32::new(hex!(
    "aa7e5b234e1d55967bf0a316395a2eab6cb3370332c0f251f0e44a5afb84fc68"
));
/// The well-known singleton launcher puzzle hash (eff07522…). A CREATE_COIN to this puzzle
/// hash is the store's launcher coin (coin_id == launcher_id == store_id).
const SINGLETON_LAUNCHER_PH: Bytes32 = Bytes32::new(hex!(
    "eff07522495060c066f66f32acc2a77e3a3e737aca8baea4d1a64ea4cdc13da9"
));

/// Domain tag for the digstore-scoped owner DISCOVERY hint — IDENTICAL to chip35's
/// (hub.dig.net), so one coinset get_coin_records_by_hint query finds stores minted by
/// EITHER the CLI or the web app. Do not change without changing chip35 in lockstep.
const DIGSTORE_OWNER_HINT_DOMAIN: &[u8] = b"dig:datastore:owner:v1";

/// Derive the digstore-scoped owner discovery hint = `sha256(DOMAIN || owner_puzzle_hash)`,
/// emitted as the FIRST (indexed) launcher memo. MUST match chip35's derivation byte-for-byte.
fn digstore_owner_hint(owner_puzzle_hash: Bytes32) -> Bytes32 {
    let mut h = chia_sha2::Sha256::new();
    h.update(DIGSTORE_OWNER_HINT_DOMAIN);
    h.update(owner_puzzle_hash);
    Bytes32::new(h.finalize())
}

/// Mint an owner-only DataLayer store, emitting the digstore-scoped owner hint as the
/// launcher coin's indexed memo. This is datalayer_driver::mint_store's body re-expressed on
/// the chia-wallet-sdk primitives, with ONE change: the launcher CREATE_COIN carries
/// `[digstore_owner_hint(owner_ph), DATASTORE_LAUNCHER_HINT]` instead of just the launcher
/// hint (datalayer_driver hardcodes a single memo and exposes no hint param). Byte-identical
/// to chip35 (hub.dig.net) so both paths are discoverable by one owner-hint query.
///
/// Single-address variant: all coins share one synthetic key and change goes back to
/// the same `owner_puzzle_hash`.
#[allow(clippy::too_many_arguments)]
fn mint_store_digstore(
    minter_synthetic_key: PublicKey,
    selected_coins: Vec<Coin>,
    root_hash: Bytes32,
    owner_puzzle_hash: Bytes32,
    delegated_puzzles: Vec<DelegatedPuzzle>,
    fee: u64,
) -> std::result::Result<SuccessResponse, DriverError> {
    // Coins are held at owner_puzzle_hash (= standard puzzle of the synthetic key), so any
    // change returns there.
    let minter_puzzle_hash = owner_puzzle_hash;
    let total_amount_from_coins = selected_coins.iter().map(|c| c.amount).sum::<u64>();
    let total_amount = fee + 1;

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(minter_synthetic_key);

    let lead_coin = selected_coins[0];
    let lead_coin_name = lead_coin.coin_id();

    for coin in selected_coins.into_iter().skip(1) {
        p2.spend(
            &mut ctx,
            coin,
            Conditions::new().assert_concurrent_spend(lead_coin_name),
        )?;
    }

    let (launch_singleton, datastore) = Launcher::new(lead_coin_name, 1).mint_datastore(
        &mut ctx,
        DataStoreMetadata {
            root_hash,
            label: None,
            description: None,
            bytes: None,
            size_proof: None,
        },
        owner_puzzle_hash.into(),
        delegated_puzzles,
    )?;

    let launch_singleton = Conditions::new().extend(
        launch_singleton
            .into_iter()
            .map(|cond| {
                if let Condition::CreateCoin(cc) = cond {
                    if cc.puzzle_hash == SINGLETON_LAUNCHER_PH {
                        let hint = ctx.memos(&[
                            digstore_owner_hint(owner_puzzle_hash),
                            DATASTORE_LAUNCHER_HINT,
                        ])?;
                        return Ok(Condition::CreateCoin(CreateCoin {
                            puzzle_hash: cc.puzzle_hash,
                            amount: cc.amount,
                            memos: hint,
                        }));
                    }
                    return Ok(Condition::CreateCoin(cc));
                }
                Ok(cond)
            })
            .collect::<std::result::Result<Vec<_>, DriverError>>()?,
    );

    let lead_coin_conditions = if total_amount_from_coins > total_amount {
        let hint = ctx.hint(minter_puzzle_hash)?;
        launch_singleton.create_coin(
            minter_puzzle_hash,
            total_amount_from_coins - total_amount,
            hint,
        )
    } else {
        launch_singleton
    };
    p2.spend(&mut ctx, lead_coin, lead_coin_conditions)?;

    Ok(SuccessResponse {
        coin_spends: ctx.take(),
        new_datastore: datastore,
    })
}

/// Multi-address variant of [`mint_store_digstore`].
///
/// `selected_coins` may span multiple HD addresses — each entry carries its own
/// `synthetic_pk` and `owner_puzzle_hash`. Change is consolidated to `change_ph`
/// (caller passes index 0's `owner_puzzle_hash`).
///
/// The lead coin's `coin_id` becomes the launcher parent id (and thus the
/// `launcher_id`). The lead coin is spent with a `StandardLayer` for its own
/// `synthetic_pk`; every other coin asserts concurrent spend with the lead coin
/// using ITS OWN `StandardLayer` (one per distinct address).
fn mint_store_digstore_multi(
    selected_coins: Vec<CoinWithKey>,
    root_hash: Bytes32,
    change_ph: Bytes32,           // index 0 owner_puzzle_hash — change consolidates here
    owner_puzzle_hash: Bytes32,   // store owner (index 0); used in the owner hint + DataStore
    delegated_puzzles: Vec<DelegatedPuzzle>,
    fee: u64,
) -> std::result::Result<SuccessResponse, DriverError> {
    assert!(!selected_coins.is_empty(), "selected_coins must be non-empty");

    let total_amount_from_coins: u64 = selected_coins.iter().map(|c| c.coin.amount).sum();
    let total_amount = fee + 1;

    let mut ctx = SpendContext::new();

    let lead = &selected_coins[0];
    let lead_coin = lead.coin;
    let lead_coin_name = lead_coin.coin_id();
    let lead_p2 = StandardLayer::new(lead.synthetic_pk);

    // Spend every non-lead coin: assert concurrent spend with the lead coin,
    // using the coin's own StandardLayer (its own synthetic_pk).
    for ck in selected_coins.iter().skip(1) {
        let p2 = StandardLayer::new(ck.synthetic_pk);
        p2.spend(
            &mut ctx,
            ck.coin,
            Conditions::new().assert_concurrent_spend(lead_coin_name),
        )?;
    }

    // Mint the singleton from the lead coin.
    let (launch_singleton, datastore) = Launcher::new(lead_coin_name, 1).mint_datastore(
        &mut ctx,
        DataStoreMetadata {
            root_hash,
            label: None,
            description: None,
            bytes: None,
            size_proof: None,
        },
        owner_puzzle_hash.into(),
        delegated_puzzles,
    )?;

    // Inject the digstore owner hint into the launcher CREATE_COIN.
    let launch_singleton = Conditions::new().extend(
        launch_singleton
            .into_iter()
            .map(|cond| {
                if let Condition::CreateCoin(cc) = cond {
                    if cc.puzzle_hash == SINGLETON_LAUNCHER_PH {
                        let hint = ctx.memos(&[
                            digstore_owner_hint(owner_puzzle_hash),
                            DATASTORE_LAUNCHER_HINT,
                        ])?;
                        return Ok(Condition::CreateCoin(CreateCoin {
                            puzzle_hash: cc.puzzle_hash,
                            amount: cc.amount,
                            memos: hint,
                        }));
                    }
                    return Ok(Condition::CreateCoin(cc));
                }
                Ok(cond)
            })
            .collect::<std::result::Result<Vec<_>, DriverError>>()?,
    );

    // Lead coin: launch the singleton + optional change back to index 0.
    let lead_conditions = if total_amount_from_coins > total_amount {
        let hint = ctx.hint(change_ph)?;
        launch_singleton.create_coin(change_ph, total_amount_from_coins - total_amount, hint)
    } else {
        launch_singleton
    };
    lead_p2.spend(&mut ctx, lead_coin, lead_conditions)?;

    Ok(SuccessResponse {
        coin_spends: ctx.take(),
        new_datastore: datastore,
    })
}

/// A built, signed mint ready to broadcast.
pub struct MintBuild {
    pub bundle: SpendBundle,
    pub launcher_id: Bytes32,
    pub datastore: DataStore,
}

/// The UNSIGNED mint: the raw coin spends plus the ids derived from them. Used
/// when extra coin spends (e.g. a DIG CAT payment) must be concatenated into the
/// SAME bundle and signed together. The owner's synthetic key signs the whole set.
pub struct MintUnsigned {
    pub coin_spends: Vec<CoinSpend>,
    pub launcher_id: Bytes32,
    pub datastore: DataStore,
}

/// Builds the UNSIGNED mint coin spends for an owner-only empty/initial store with
/// `root`. `unspent` are the wallet's spendable XCH coins; `fee` in mojos. The
/// launcher id is derived intact from the mint; the caller signs (alone, or after
/// concatenating other coin spends into one bundle).
///
/// Single-address variant (kept for tests + the single-key `build_mint` wrapper).
pub fn build_mint_unsigned(
    keys: &WalletKeys,
    unspent: &[Coin],
    root: Bytes32,
    fee: u64,
) -> Result<MintUnsigned> {
    let selected = select_coins(unspent, fee + 1)
        .map_err(|e| ChainError::Chain(format!("select_coins: {e}")))?;
    let SuccessResponse {
        coin_spends,
        new_datastore,
    } = mint_store_digstore(
        keys.synthetic_pk,
        selected,
        root,
        keys.owner_puzzle_hash,
        vec![],
        fee,
    )
    .map_err(|e| ChainError::Chain(format!("mint_store: {e}")))?;
    let launcher_id = new_datastore.info.launcher_id;
    Ok(MintUnsigned {
        coin_spends,
        launcher_id,
        datastore: new_datastore,
    })
}

/// Multi-address variant of [`build_mint_unsigned`].
///
/// `all_coins` contains ALL XCH coins across scanned HD addresses, each tagged with
/// the address's `synthetic_pk` and `owner_puzzle_hash`. This function greedily
/// selects enough coins to cover `fee + 1` mojos (using `select_coins` on the flat
/// coin list), tags the selected set with their per-address keys, and builds the
/// unsigned mint spends. Change consolidates to `change_ph` (index 0's
/// `owner_puzzle_hash`). The store owner (in DataStore metadata) is also `change_ph`.
pub fn build_mint_unsigned_multi(
    all_coins: &[CoinWithKey],
    change_ph: Bytes32,
    root: Bytes32,
    fee: u64,
) -> Result<MintUnsigned> {
    // Flatten to bare Coin slice for coin selection.
    let flat: Vec<Coin> = all_coins.iter().map(|c| c.coin).collect();
    let selected_flat = select_coins(&flat, fee + 1)
        .map_err(|e| ChainError::Chain(format!("select_coins: {e}")))?;

    // Re-attach per-address keys to the selected coins.
    // Build a lookup: coin_id -> CoinWithKey (parent+ph identify a coin uniquely).
    let selected_ids: std::collections::HashSet<Bytes32> =
        selected_flat.iter().map(|c| c.coin_id()).collect();
    let selected: Vec<CoinWithKey> = all_coins
        .iter()
        .filter(|c| selected_ids.contains(&c.coin.coin_id()))
        .cloned()
        .collect();

    if selected.is_empty() {
        return Err(ChainError::Chain(
            "select_coins returned empty set for multi-address mint".into(),
        ));
    }

    let SuccessResponse {
        coin_spends,
        new_datastore,
    } = mint_store_digstore_multi(selected, root, change_ph, change_ph, vec![], fee)
        .map_err(|e| ChainError::Chain(format!("mint_store_multi: {e}")))?;

    let launcher_id = new_datastore.info.launcher_id;
    Ok(MintUnsigned {
        coin_spends,
        launcher_id,
        datastore: new_datastore,
    })
}

/// Flatten the XCH coins across all scanned addresses into `Vec<CoinWithKey>`,
/// suitable for passing to [`build_mint_unsigned_multi`].
pub fn coins_with_keys_from_wallet(
    w: &crate::wallet::ScannedWallet,
) -> Vec<CoinWithKey> {
    w.addrs
        .iter()
        .flat_map(|a| {
            a.xch.iter().map(move |coin| CoinWithKey {
                coin: *coin,
                synthetic_pk: a.keys.synthetic_pk,
                owner_puzzle_hash: a.keys.owner_puzzle_hash,
            })
        })
        .collect()
}

/// Builds + signs a mint of an owner-only empty/initial store with `root`.
/// `unspent` are the wallet's spendable XCH coins; `fee` in mojos.
pub fn build_mint(
    keys: &WalletKeys,
    unspent: &[Coin],
    root: Bytes32,
    fee: u64,
) -> Result<MintBuild> {
    let MintUnsigned {
        coin_spends,
        launcher_id,
        datastore,
    } = build_mint_unsigned(keys, unspent, root, fee)?;
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        false,
    )
    .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    let bundle = SpendBundle::new(coin_spends, signature);
    Ok(MintBuild {
        bundle,
        launcher_id,
        datastore,
    })
}

/// A built, signed store-root update ready to broadcast.
pub struct UpdateBuild {
    pub bundle: SpendBundle,
    pub new_coin_id: Bytes32,
    pub datastore: DataStore,
}

/// The UNSIGNED root update: raw coin spends (singleton + fee) plus derived ids.
/// Used to concatenate a DIG CAT payment into the SAME bundle before signing.
pub struct UpdateUnsigned {
    pub coin_spends: Vec<CoinSpend>,
    pub new_coin_id: Bytes32,
    pub datastore: DataStore,
}

/// Builds the UNSIGNED owner-authorized update of `store`'s root to `new_root`
/// (singleton spend + XCH fee spend). `fee_coins` are the wallet's spendable XCH
/// coins for the fee; `fee` mojos. The caller signs (alone or after concatenating
/// other coin spends into one bundle).
///
/// Single-address variant (all fee coins share one synthetic key).
pub fn build_update_unsigned(
    keys: &WalletKeys,
    store: DataStore,
    new_root: Bytes32,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<UpdateUnsigned> {
    let SuccessResponse {
        coin_spends: update_spends,
        new_datastore,
    } = update_store_metadata(
        store,
        new_root,
        None,
        None,
        None,
        None,
        DataStoreInnerSpend::Owner(keys.synthetic_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_metadata: {e}")))?;

    let selected = select_coins(fee_coins, fee)
        .map_err(|e| ChainError::Chain(format!("select_coins (fee): {e}")))?;
    let coin_ids: Vec<Bytes32> = selected.iter().map(|c| c.coin_id()).collect();
    let mut coin_spends = add_fee(&keys.synthetic_pk, &selected, &coin_ids, fee)
        .map_err(|e| ChainError::Chain(format!("add_fee: {e}")))?;
    coin_spends.extend(update_spends);

    let new_coin_id = new_datastore.coin.coin_id();
    Ok(UpdateUnsigned {
        coin_spends,
        new_coin_id,
        datastore: new_datastore,
    })
}

/// Multi-address variant of [`build_update_unsigned`].
///
/// `owner_pk` is index 0's `synthetic_pk` — used to authorize the singleton update
/// (the store was minted by that key). `all_fee_coins` may span multiple HD
/// addresses; each is spent under its own `synthetic_pk`, with change returning to
/// `change_ph` (index 0's `owner_puzzle_hash`). If `fee == 0`, no fee coins are
/// selected and `add_fee` is skipped.
pub fn build_update_unsigned_multi(
    owner_pk: PublicKey,
    store: DataStore,
    new_root: Bytes32,
    all_fee_coins: &[CoinWithKey],
    fee: u64,
) -> Result<UpdateUnsigned> {
    let SuccessResponse {
        coin_spends: update_spends,
        new_datastore,
    } = update_store_metadata(
        store,
        new_root,
        None,
        None,
        None,
        None,
        DataStoreInnerSpend::Owner(owner_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_metadata: {e}")))?;

    let mut coin_spends = Vec::new();
    if fee > 0 {
        let flat: Vec<Coin> = all_fee_coins.iter().map(|c| c.coin).collect();
        let selected_flat = select_coins(&flat, fee)
            .map_err(|e| ChainError::Chain(format!("select_coins (fee): {e}")))?;
        let selected_ids: std::collections::HashSet<Bytes32> =
            selected_flat.iter().map(|c| c.coin_id()).collect();
        let selected: Vec<CoinWithKey> = all_fee_coins
            .iter()
            .filter(|c| selected_ids.contains(&c.coin.coin_id()))
            .cloned()
            .collect();

        // add_fee takes a single synthetic_pk for all coins; we spend each coin
        // under its own StandardLayer to honour multi-address.
        // Build per-coin fee spends manually using StandardLayer (same approach as
        // mint_store_digstore_multi: each coin asserts concurrent spend, but here
        // the fee instruction comes from add_fee). We use the first selected coin's
        // key with add_fee (the fee reservation is on that coin), then spend the
        // rest as assert_concurrent spends under their own keys.
        //
        // For the simple (and typical) case — one fee coin or all from one address —
        // this collapses to the original behavior.
        let lead = &selected[0];
        let lead_flat: Vec<Coin> = vec![lead.coin];
        let lead_coin_ids: Vec<Bytes32> = vec![lead.coin.coin_id()];
        let mut fee_spends = add_fee(
            &lead.synthetic_pk,
            &lead_flat,
            &lead_coin_ids,
            fee,
        )
        .map_err(|e| ChainError::Chain(format!("add_fee: {e}")))?;

        // Spend remaining fee coins: assert concurrent with the lead fee coin.
        if selected.len() > 1 {
            let lead_coin_name = lead.coin.coin_id();
            let mut ctx = SpendContext::new();
            for ck in selected.iter().skip(1) {
                let p2 = StandardLayer::new(ck.synthetic_pk);
                p2.spend(
                    &mut ctx,
                    ck.coin,
                    Conditions::new().assert_concurrent_spend(lead_coin_name),
                )
                .map_err(|e| {
                    ChainError::Chain(format!("fee coin extra spend: {e}"))
                })?;
            }
            fee_spends.extend(ctx.take());
        }

        coin_spends.extend(fee_spends);
    }
    coin_spends.extend(update_spends);

    let new_coin_id = new_datastore.coin.coin_id();
    Ok(UpdateUnsigned {
        coin_spends,
        new_coin_id,
        datastore: new_datastore,
    })
}

/// Builds + signs an owner-authorized update of `store`'s root to `new_root`.
/// `fee_coins` are the wallet's spendable XCH coins for the fee; `fee` mojos.
pub fn build_update(
    keys: &WalletKeys,
    store: DataStore,
    new_root: Bytes32,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<UpdateBuild> {
    let UpdateUnsigned {
        coin_spends,
        new_coin_id,
        datastore,
    } = build_update_unsigned(keys, store, new_root, fee_coins, fee)?;
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        false,
    )
    .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    let bundle = SpendBundle::new(coin_spends, signature);
    Ok(UpdateBuild {
        bundle,
        new_coin_id,
        datastore,
    })
}

/// Reconstructs the current unspent datastore singleton for `launcher_id` using
/// only coinset reads (coin records + puzzle/solution), following the singleton
/// lineage. No P2P peer required. Owner-only stores carry no delegated puzzles.
///
/// `DataStore::from_spend(ctx, spend, delegated)` returns the CHILD datastore
/// created by spending `spend.coin`, so we walk launcher -> eve -> ... forward
/// until we reach a singleton coin that is still unspent.
pub async fn sync_datastore(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<DataStore> {
    let mut ctx = SpendContext::new();

    // The launcher coin is spent to create the eve singleton.
    let launcher = chain
        .coin_record(launcher_id)
        .await?
        .ok_or_else(|| ChainError::Chain(format!("launcher coin {launcher_id:?} not found")))?;
    if !launcher.spent {
        return Err(ChainError::Chain(
            "launcher coin is unspent (store not minted yet)".into(),
        ));
    }
    let launcher_spend = chain
        .coin_spend(launcher_id, launcher.spent_block_index)
        .await?
        .ok_or_else(|| ChainError::Chain("launcher spend not found".into()))?;

    let mut store = DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &launcher_spend, &[])
        .map_err(|e| ChainError::Chain(format!("parse eve store: {e}")))?
        .ok_or_else(|| ChainError::Chain("launcher spend is not a datastore".into()))?;

    // Walk forward until the singleton coin is unspent.
    const MAX_HOPS: u32 = 100_000;
    let mut hops = 0u32;
    loop {
        hops += 1;
        if hops > MAX_HOPS {
            return Err(ChainError::Chain(format!(
                "singleton chain exceeded {MAX_HOPS} hops; possible cycle or corrupt chain data"
            )));
        }
        let coin_id = store.coin.coin_id();
        let rec = chain
            .coin_record(coin_id)
            .await?
            .ok_or_else(|| ChainError::Chain(format!("singleton coin {coin_id:?} not found")))?;
        if !rec.spent {
            return Ok(store); // current, unspent singleton
        }
        let spend = chain
            .coin_spend(coin_id, rec.spent_block_index)
            .await?
            .ok_or_else(|| ChainError::Chain("singleton spend not found".into()))?;
        let delegated: Vec<DelegatedPuzzle> = store.info.delegated_puzzles.clone();
        store = DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &spend, &delegated)
            .map_err(|e| ChainError::Chain(format!("parse next store: {e}")))?
            .ok_or_else(|| ChainError::Chain("singleton spend did not yield a store".into()))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_wallet_keys;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn build_update_errors_with_empty_fee_coins_and_nonzero_fee() {
        // Constructing a real DataStore requires going through mint_store; skip
        // that here and just confirm select_coins rejects the empty coin list
        // before we even reach update_store_metadata.  We do this by calling
        // build_mint first to get a valid DataStore, then immediately feeding it
        // to build_update with no fee coins.
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1_000_000);
        let mb = build_mint(&keys, &[coin], Bytes32::default(), 0).unwrap();
        // Now call build_update with an empty fee_coins slice and a nonzero fee.
        let result = build_update(&keys, mb.datastore, Bytes32::new([1u8; 32]), &[], 1_000);
        assert!(result.is_err(), "expected error with empty fee coins");
    }

    #[test]
    fn build_mint_produces_signed_bundle_and_launcher() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        // A synthetic funding coin at the owner puzzle hash (mint_store builds
        // the spend purely; it does not check on-chain existence).
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1_000_000);
        let mb = build_mint(&keys, &[coin], Bytes32::default(), 1_000).unwrap();
        assert!(!mb.bundle.coin_spends.is_empty());
        assert_ne!(mb.launcher_id, Bytes32::default()); // a real launcher id was derived
    }

    #[test]
    fn build_mint_errors_when_insufficient_coins() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1); // < fee+1
        assert!(build_mint(&keys, &[coin], Bytes32::default(), 1_000).is_err());
    }
}

/// Read the launcher's current on-chain root by syncing its singleton lineage
/// over `chain` and returning the latest metadata root. Errors (propagated from
/// `sync_datastore`) mean the chain could not be read — callers MUST fail closed.
pub async fn current_root(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<Bytes32> {
    let store = sync_datastore(chain, launcher_id).await?;
    Ok(store.info.metadata.root_hash)
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::Coinset;

    fn launcher_bytes32() -> Bytes32 {
        let raw = hex::decode("cf915cbaac0755db8c79b1b2e3b2eadf14d14f7246bb7e05d951802cd273211c")
            .expect("valid hex");
        let arr: [u8; 32] = raw.try_into().expect("32 bytes");
        Bytes32::new(arr)
    }

    // Structural test: no peer, no network. A launcher id with no coin record
    // surfaces a "not found" error rather than panicking.
    #[tokio::test]
    async fn sync_errors_when_launcher_not_found() {
        let chain = MockChain::default();
        let err = sync_datastore(&chain, Bytes32::default())
            .await
            .unwrap_err();
        match err {
            ChainError::Chain(msg) => assert!(msg.contains("not found"), "got: {msg}"),
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    // current_root propagates the same Chain error when the launcher is absent.
    // This mirrors sync_errors_when_launcher_not_found with the same MockChain
    // fixture — verifying error propagation: current_root delegates to
    // sync_datastore and fails closed on any chain read error. The happy-path
    // (successful root retrieval) is covered by the ignored live test
    // `sync_live_minted_store`.
    #[tokio::test]
    async fn current_root_fails_closed_when_launcher_not_found() {
        let chain = MockChain::default();
        let err = current_root(&chain, Bytes32::default()).await.unwrap_err();
        match err {
            ChainError::Chain(msg) => assert!(msg.contains("not found"), "got: {msg}"),
            other => panic!("expected Chain error, got {other:?}"),
        }
    }

    // Live build-only test: syncs the real mainnet store, builds (but does NOT
    // push) an owner-authorized root update. Free — no XCH spent.
    // Requires a wallet mnemonic in ../../.testcredentials (gitignored).
    // Run with:
    //   cargo test -p digstore-chain --lib -- --ignored build_update_live_no_broadcast --nocapture
    #[tokio::test]
    #[ignore]
    async fn build_update_live_no_broadcast() {
        use crate::keys::derive_wallet_keys;
        let chain = Coinset::mainnet();
        let launcher = launcher_bytes32();
        let store = sync_datastore(&chain, launcher).await.unwrap();
        // Read the test wallet mnemonic from the gitignored .testcredentials
        // file at runtime.  cargo runs tests with the crate dir as CWD, so
        // ../../.testcredentials reaches the repo root from crates/digstore-chain.
        let phrase = std::fs::read_to_string("../../.testcredentials").unwrap();
        let keys = derive_wallet_keys(phrase.trim()).unwrap();
        let fee_coins = chain.unspent_coins(keys.owner_puzzle_hash).await.unwrap();
        let new_root = Bytes32::new([7u8; 32]);
        let built = build_update(&keys, store, new_root, &fee_coins, 1_000).unwrap();
        assert!(!built.bundle.coin_spends.is_empty());
        assert_eq!(built.datastore.info.metadata.root_hash, new_root);
        println!("built update; new coin id = {:?}", built.new_coin_id);
    }

    // Live read-only test against the real minted mainnet store. Free (no spend).
    // Run with:
    //   cargo test -p digstore-chain --lib -- --ignored sync_live_minted_store --nocapture
    #[tokio::test]
    #[ignore]
    async fn sync_live_minted_store() {
        let chain = Coinset::mainnet();
        let launcher = launcher_bytes32();
        let store = sync_datastore(&chain, launcher).await.unwrap();
        assert_eq!(store.info.launcher_id, launcher);
        // minted with empty root, never updated:
        assert_eq!(store.info.metadata.root_hash, Bytes32::default());
        println!("synced store coin id = {:?}", store.coin.coin_id());
    }
}
