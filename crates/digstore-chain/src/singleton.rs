//! Build + sign Chia datastore singleton spends (mint + update). Pure: callers
//! fetch unspent coins via `ChainReads` and broadcast the returned bundle via
//! `ChainReads::push`. Verified on mainnet in the Phase-0 prototype.

use crate::coinset::ChainReads;
use crate::error::{ChainError, Result};
use crate::keys::WalletKeys;
use chia_wallet_sdk::driver::{DriverError, Launcher, SpendContext, StandardLayer};
use chia_wallet_sdk::types::{conditions::CreateCoin, Condition, Conditions};
use datalayer_driver::{
    add_fee, admin_delegated_puzzle_from_key, melt_store, oracle_delegated_puzzle as dl_oracle_dp,
    sign_coin_spends, update_store_metadata, update_store_ownership, Bytes32, Coin, CoinSpend,
    DataStoreInnerSpend, DataStoreMetadata, SpendBundle, SuccessResponse,
};

/// Re-export the datalayer types that appear in this module's public builder
/// signatures, so downstream crates (the in-process `dig-wallet`) can name a
/// `DataStore` / `DelegatedPuzzle` / `PublicKey` WITHOUT taking a direct
/// `datalayer-driver` dependency — they speak the store-spend surface through
/// `digstore-chain` (the byte-mirror of chip35), keeping the one-builder-source rule.
pub use datalayer_driver::{DataStore, DelegatedPuzzle, PublicKey};

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
    label: Option<String>,
    description: Option<String>,
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
            label,
            description,
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
#[allow(clippy::too_many_arguments)]
fn mint_store_digstore_multi(
    selected_coins: Vec<CoinWithKey>,
    root_hash: Bytes32,
    label: Option<String>,
    description: Option<String>,
    change_ph: Bytes32, // index 0 owner_puzzle_hash — change consolidates here
    owner_puzzle_hash: Bytes32, // store owner (index 0); used in the owner hint + DataStore
    delegated_puzzles: Vec<DelegatedPuzzle>,
    fee: u64,
) -> std::result::Result<SuccessResponse, DriverError> {
    assert!(
        !selected_coins.is_empty(),
        "selected_coins must be non-empty"
    );

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
            label,
            description,
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
    label: Option<String>,
    description: Option<String>,
    fee: u64,
) -> Result<MintUnsigned> {
    let selected = crate::selection::select_xch_coins(unspent, fee + 1)?;
    let SuccessResponse {
        coin_spends,
        new_datastore,
    } = mint_store_digstore(
        keys.synthetic_pk,
        selected,
        root,
        label,
        description,
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
/// the address's `synthetic_pk` and `owner_puzzle_hash`. This function selects enough
/// coins to cover `fee + 1` mojos via the shared capped selector
/// ([`crate::selection::select_xch_coins`] — high-value-first, at most
/// [`crate::selection::COIN_CAP`] inputs), tags the selected set with their
/// per-address keys, and builds the unsigned mint spends. When the wallet is too
/// fragmented to cover the mint within the cap it returns
/// [`ChainError::NeedsConsolidation`] (the CLI consolidates + retries). Change
/// consolidates to `change_ph` (index 0's `owner_puzzle_hash`). The store owner (in
/// DataStore metadata) is also `change_ph`.
pub fn build_mint_unsigned_multi(
    all_coins: &[CoinWithKey],
    change_ph: Bytes32,
    root: Bytes32,
    label: Option<String>,
    description: Option<String>,
    fee: u64,
) -> Result<MintUnsigned> {
    // Flatten to bare Coin slice for coin selection.
    let flat: Vec<Coin> = all_coins.iter().map(|c| c.coin).collect();
    let selected_flat = crate::selection::select_xch_coins(&flat, fee + 1)?;

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
            "coin selection returned empty set for multi-address mint".into(),
        ));
    }

    let SuccessResponse {
        coin_spends,
        new_datastore,
    } = mint_store_digstore_multi(
        selected,
        root,
        label,
        description,
        change_ph,
        change_ph,
        vec![],
        fee,
    )
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
pub fn coins_with_keys_from_wallet(w: &crate::wallet::ScannedWallet) -> Vec<CoinWithKey> {
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
    label: Option<String>,
    description: Option<String>,
    fee: u64,
) -> Result<MintBuild> {
    let MintUnsigned {
        coin_spends,
        launcher_id,
        datastore,
    } = build_mint_unsigned(keys, unspent, root, label, description, fee)?;
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
    label: Option<String>,
    description: Option<String>,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<UpdateUnsigned> {
    // `update_store_metadata` REPLACES the singleton metadata, so label/description must be
    // re-sent on every update or they'd be cleared. Callers pass the values persisted at init.
    let SuccessResponse {
        coin_spends: update_spends,
        new_datastore,
    } = update_store_metadata(
        store,
        new_root,
        label,
        description,
        None,
        None,
        DataStoreInnerSpend::Owner(keys.synthetic_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_metadata: {e}")))?;

    let selected = crate::selection::select_xch_coins(fee_coins, fee)?;
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
    label: Option<String>,
    description: Option<String>,
    all_fee_coins: &[CoinWithKey],
    fee: u64,
) -> Result<UpdateUnsigned> {
    // Re-send label/description (update REPLACES metadata; see build_update_unsigned).
    let SuccessResponse {
        coin_spends: update_spends,
        new_datastore,
    } = update_store_metadata(
        store,
        new_root,
        label,
        description,
        None,
        None,
        DataStoreInnerSpend::Owner(owner_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_metadata: {e}")))?;

    let mut coin_spends = Vec::new();
    if fee > 0 {
        let flat: Vec<Coin> = all_fee_coins.iter().map(|c| c.coin).collect();
        let selected_flat = crate::selection::select_xch_coins(&flat, fee)?;
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
        let mut fee_spends = add_fee(&lead.synthetic_pk, &lead_flat, &lead_coin_ids, fee)
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
                .map_err(|e| ChainError::Chain(format!("fee coin extra spend: {e}")))?;
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

/// The [`DelegatedPuzzle::Writer`] for a writer-delegate synthetic key — the
/// on-chain authorization the store owner curries in (via `updateStoreOwnership`)
/// so this key can advance the metadata root WITHOUT being the owner master seed.
///
/// A re-export of `datalayer_driver::writer_delegated_puzzle_from_key` so callers
/// (the CLI deploy-token path, the hub Teams "Deployer") name the writer delegate
/// from one place. The owner adds this `DelegatedPuzzle` to the store's delegated
/// set; thereafter `build_update_unsigned_writer` can sign a root advance with the
/// writer key alone. The writer can change the metadata root only — it can NOT
/// change ownership or melt the store (that is the owner's Admin/Owner authority).
pub fn writer_delegated_puzzle(writer_synthetic_pk: PublicKey) -> DelegatedPuzzle {
    datalayer_driver::writer_delegated_puzzle_from_key(&writer_synthetic_pk)
}

/// The [`DelegatedPuzzle::Admin`] for an admin-delegate synthetic key — the on-chain
/// authorization the store owner curries in (via [`build_update_ownership_unsigned`])
/// so this key can advance the metadata root AND re-delegate the store's delegated set
/// (add/remove writers/admins), WITHOUT being able to transfer ownership or melt the
/// store (those stay the owner's authority). The DIG Browser / hub Teams "Admin" role.
///
/// A thin re-export of `datalayer_driver::admin_delegated_puzzle_from_key` (byte-mirror
/// of chip35's `DelegatedPuzzle::Admin`) so callers name the admin delegate from one
/// place. Adds NO new on-chain spend type — it is the same datalayer/chip35 puzzle.
pub fn admin_delegated_puzzle(admin_synthetic_pk: PublicKey) -> DelegatedPuzzle {
    admin_delegated_puzzle_from_key(&admin_synthetic_pk)
}

/// The [`DelegatedPuzzle::Oracle`] for an oracle reader — anyone may spend the store
/// in oracle mode by paying `oracle_fee` mojos to `oracle_puzzle_hash`, without owner
/// authorization (a public, paid read commitment). Re-export of
/// `datalayer_driver::oracle_delegated_puzzle` (byte-mirror of chip35's
/// `DelegatedPuzzle::Oracle`), so the wallet/CLI/hub derive the oracle delegate
/// identically. Adds NO new on-chain spend type.
pub fn oracle_delegated_puzzle(oracle_puzzle_hash: Bytes32, oracle_fee: u64) -> DelegatedPuzzle {
    dl_oracle_dp(oracle_puzzle_hash, oracle_fee)
}

/// Builds the UNSIGNED root update of `store` to `new_root` authorized by a WRITER
/// DELEGATE key (a revocable CI deploy key) rather than the owner master seed
/// (#17). The store MUST already carry this writer's [`writer_delegated_puzzle`] in
/// its delegated set (the owner pre-authorizes it via `updateStoreOwnership` — the
/// hub Teams "Deployer" flow); `sync_datastore` preserves that delegated set, so a
/// reconstructed `store` is ready for a writer spend.
///
/// The singleton spend is built with [`DataStoreInnerSpend::Writer`], so it is the
/// writer's synthetic key — NOT the owner key — that authorizes the metadata
/// update. A writer spend canNOT change the owner or melt the store (the writer
/// puzzle only permits a metadata update), so a leaked/abused deploy key can never
/// take over or destroy the store — the owner revokes it by re-running
/// `updateStoreOwnership` without that delegated puzzle.
///
/// The XCH `fee` is still drawn from the wallet's `fee_coins` and signed by the
/// wallet key — the writer authorizes the singleton spend; the wallet pays the
/// network fee + the DIG payment (concatenated by the caller, exactly as the owner
/// path does). `writer_synthetic_pk` authorizes the singleton; `fee_keys` authorize
/// the fee coins.
///
/// **Pure: does NOT sign or broadcast.** The caller signs with BOTH the writer key
/// and the fee/DIG-coin keys (one aggregated signature over the whole bundle).
#[allow(clippy::too_many_arguments)]
pub fn build_update_unsigned_writer(
    writer_synthetic_pk: PublicKey,
    store: DataStore,
    new_root: Bytes32,
    label: Option<String>,
    description: Option<String>,
    fee_keys: &WalletKeys,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<UpdateUnsigned> {
    // Writer-authorized metadata update (REPLACES metadata, so re-send label/desc).
    let SuccessResponse {
        coin_spends: update_spends,
        new_datastore,
    } = update_store_metadata(
        store,
        new_root,
        label,
        description,
        None,
        None,
        DataStoreInnerSpend::Writer(writer_synthetic_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_metadata (writer): {e}")))?;

    let mut coin_spends = Vec::new();
    if fee > 0 {
        let selected = crate::selection::select_xch_coins(fee_coins, fee)?;
        let coin_ids: Vec<Bytes32> = selected.iter().map(|c| c.coin_id()).collect();
        let fee_spends = add_fee(&fee_keys.synthetic_pk, &selected, &coin_ids, fee)
            .map_err(|e| ChainError::Chain(format!("add_fee: {e}")))?;
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
    label: Option<String>,
    description: Option<String>,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<UpdateBuild> {
    let UpdateUnsigned {
        coin_spends,
        new_coin_id,
        datastore,
    } = build_update_unsigned(keys, store, new_root, label, description, fee_coins, fee)?;
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

/// A built, signed store melt ready to broadcast.
pub struct MeltBuild {
    pub bundle: SpendBundle,
}

/// The UNSIGNED melt: the raw singleton spend that destroys the store. Used to
/// concatenate an XCH fee spend into the SAME bundle before signing (the melt itself
/// reserves the singleton's 1 mojo as fee, so a melt needs no extra payment).
pub struct MeltUnsigned {
    pub coin_spends: Vec<CoinSpend>,
}

/// Builds the UNSIGNED owner-authorized melt of `store` — destroys the singleton,
/// permanently retiring the on-chain store (no further capsules can ever be anchored
/// under its launcher id). Wraps `datalayer_driver::melt_store` (byte-mirror of
/// chip35's `meltStore`), so this introduces NO new on-chain spend type — it is the
/// same melt the CLI/hub use. The melt reserves the singleton's 1 mojo as the network
/// fee, so no DIG payment and no extra fee coin are required.
///
/// **Pure: does NOT sign or broadcast.** The caller signs with the owner key.
pub fn build_melt_unsigned(keys: &WalletKeys, store: DataStore) -> Result<MeltUnsigned> {
    let coin_spends = melt_store(store, keys.synthetic_pk)
        .map_err(|e| ChainError::Chain(format!("melt_store: {e}")))?;
    Ok(MeltUnsigned { coin_spends })
}

/// Builds + signs an owner-authorized melt of `store`. See [`build_melt_unsigned`].
pub fn build_melt(keys: &WalletKeys, store: DataStore) -> Result<MeltBuild> {
    let MeltUnsigned { coin_spends } = build_melt_unsigned(keys, store)?;
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        false,
    )
    .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    Ok(MeltBuild {
        bundle: SpendBundle::new(coin_spends, signature),
    })
}

/// A built, signed ownership/delegation update ready to broadcast.
pub struct OwnershipBuild {
    pub bundle: SpendBundle,
    pub new_coin_id: Bytes32,
    pub datastore: DataStore,
}

/// The UNSIGNED ownership/delegation update: the raw singleton spend that re-targets
/// the store's owner puzzle hash and/or its delegated-puzzle set, plus derived ids.
/// Used to concatenate an XCH fee spend into the SAME bundle before signing.
pub struct OwnershipUnsigned {
    pub coin_spends: Vec<CoinSpend>,
    pub new_coin_id: Bytes32,
    pub datastore: DataStore,
}

/// Builds the UNSIGNED owner-authorized update of `store`'s OWNERSHIP — re-targets the
/// owner puzzle hash to `new_owner_ph` and REPLACES the delegated-puzzle set with
/// `new_delegated_puzzles` (admin/writer/oracle delegates derived via
/// [`admin_delegated_puzzle`]/[`writer_delegated_puzzle`]/[`oracle_delegated_puzzle`]).
///
/// This is the ONE on-chain primitive behind BOTH delegation-management (keep
/// `new_owner_ph == store.info.owner_puzzle_hash`, change the delegated set — the
/// Teams admin/writer/oracle flow #43/#17) AND ownership transfer (set `new_owner_ph`
/// to the recipient's p2 puzzle hash). Wraps `datalayer_driver::update_store_ownership`
/// (byte-mirror of chip35's `updateStoreOwnership`), so it adds NO new on-chain spend
/// type. The delegated set REPLACES the prior set, so callers re-send every delegate
/// they want to keep (this is also how a delegate is revoked — omit it).
///
/// An optional XCH `fee` is drawn from `fee_coins` (the wallet's spendable XCH) and
/// spent by the owner key. **Pure: does NOT sign or broadcast.**
pub fn build_update_ownership_unsigned(
    keys: &WalletKeys,
    store: DataStore,
    new_owner_ph: Bytes32,
    new_delegated_puzzles: Vec<DelegatedPuzzle>,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<OwnershipUnsigned> {
    let SuccessResponse {
        coin_spends: ownership_spends,
        new_datastore,
    } = update_store_ownership(
        store,
        new_owner_ph,
        new_delegated_puzzles,
        DataStoreInnerSpend::Owner(keys.synthetic_pk),
    )
    .map_err(|e| ChainError::Chain(format!("update_store_ownership: {e}")))?;

    let mut coin_spends = Vec::new();
    if fee > 0 {
        let selected = crate::selection::select_xch_coins(fee_coins, fee)?;
        let coin_ids: Vec<Bytes32> = selected.iter().map(|c| c.coin_id()).collect();
        let fee_spends = add_fee(&keys.synthetic_pk, &selected, &coin_ids, fee)
            .map_err(|e| ChainError::Chain(format!("add_fee: {e}")))?;
        coin_spends.extend(fee_spends);
    }
    coin_spends.extend(ownership_spends);

    let new_coin_id = new_datastore.coin.coin_id();
    Ok(OwnershipUnsigned {
        coin_spends,
        new_coin_id,
        datastore: new_datastore,
    })
}

/// Builds + signs an owner-authorized ownership/delegation update. See
/// [`build_update_ownership_unsigned`].
pub fn build_update_ownership(
    keys: &WalletKeys,
    store: DataStore,
    new_owner_ph: Bytes32,
    new_delegated_puzzles: Vec<DelegatedPuzzle>,
    fee_coins: &[Coin],
    fee: u64,
) -> Result<OwnershipBuild> {
    let OwnershipUnsigned {
        coin_spends,
        new_coin_id,
        datastore,
    } = build_update_ownership_unsigned(
        keys,
        store,
        new_owner_ph,
        new_delegated_puzzles,
        fee_coins,
        fee,
    )?;
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        false,
    )
    .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    Ok(OwnershipBuild {
        bundle: SpendBundle::new(coin_spends, signature),
        new_coin_id,
        datastore,
    })
}

/// The classified terminal state of a datastore singleton lineage.
///
/// The lineage walk (launcher → eve → … → tip) has exactly THREE well-formed
/// terminal outcomes, which the legacy [`sync_datastore`] collapses into one
/// stringly-typed [`ChainError::Chain`]. Distinguishing them is CUSTODY-CRITICAL:
/// a downstream consumer (dig-node store-melt propagation, #1316) authorizes an
/// IRREVERSIBLE local-data delete only on a genuine [`Melted`](Self::Melted), and
/// MUST NOT delete on a [`NeverMinted`](Self::NeverMinted) or an
/// unreachable/corrupt chain (which surfaces as `Err`, NEVER as `Melted`).
///
/// The invariant that makes this safe: **corrupt is never melt.** A `Melted` is
/// returned ONLY when a singleton coin that genuinely parsed as a datastore was
/// SPENT and its spend, parsed by `DataStore::from_spend`, yielded no datastore
/// child (`Ok(None)`) — the exact on-chain signature of an owner-authorized melt.
/// Any unreadable coin record, missing/unparseable spend, or a launcher whose
/// spend is not a datastore at all stays an `Err`.
#[derive(Debug)]
pub enum SingletonTerminal {
    /// The launcher was spent and the forward walk reached an UNSPENT tip — the
    /// store is live and spendable. Carries the current [`DataStore`] (boxed: it is
    /// far larger than the other variants, so boxing keeps the enum compact).
    Live(Box<DataStore>),
    /// The launcher coin exists on-chain but is still UNSPENT: the store was
    /// never minted (no eve singleton was ever created). NOT a melt.
    NeverMinted,
    /// The launcher was spent and the walk reached a singleton coin that was
    /// SPENT to a spend yielding no datastore child — an owner-authorized MELT,
    /// permanently retiring the store.
    Melted {
        /// The last on-chain root observed before the melt (the metadata root of
        /// the singleton generation that was melt-spent). Lets a consumer record
        /// what content the retired store last anchored.
        last_root: Bytes32,
        /// The block height at which the terminal (melt) spend was confirmed.
        /// Downstream MUST enforce a burial depth against this height before
        /// acting irreversibly: a reorg that un-melts the store AFTER a delete is
        /// permanent data loss.
        melt_spent_height: u32,
    },
}

/// Walk the singleton lineage for `launcher_id` and classify its terminal state.
///
/// This is the ONE forward-walk traversal shared by [`sync_datastore`] (which maps
/// the non-`Live` terminals back to its legacy error strings for byte-identical
/// behaviour) and [`classify_singleton`] (which surfaces the typed enum). Keeping a
/// single walker upholds the canonical-crate rule: a second lineage walker anywhere
/// in the ecosystem is a byte-drift bug on a custody path.
///
/// `DataStore::from_spend(ctx, spend, delegated)` returns the CHILD datastore created
/// by spending `spend.coin`, so we walk launcher → eve → … forward until either a
/// singleton coin is still unspent (`Live`) or a spent coin's spend yields no child
/// (`Melted`). Every genuine chain-read failure and every unparseable spend returns
/// `Err` — corrupt is never `Melted`.
async fn walk_singleton_terminal(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
) -> Result<SingletonTerminal> {
    let mut ctx = SpendContext::new();

    // The launcher coin is spent to create the eve singleton.
    let launcher = chain
        .coin_record(launcher_id)
        .await?
        .ok_or_else(|| ChainError::Chain(format!("launcher coin {launcher_id:?} not found")))?;
    if !launcher.spent {
        // Launcher exists but is unspent: the store was never minted. This is a
        // well-formed terminal state, NOT an error and NOT a melt.
        return Ok(SingletonTerminal::NeverMinted);
    }
    let launcher_spend = chain
        .coin_spend(launcher_id, launcher.spent_block_index)
        .await?
        .ok_or_else(|| ChainError::Chain("launcher spend not found".into()))?;

    // A launcher spend that does NOT yield a datastore is a corrupt / non-store
    // launcher — NOT a melt. Only a spend of a coin that already parsed as a
    // datastore (in the loop below) can be a melt.
    let mut store = DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &launcher_spend, &[])
        .map_err(|e| ChainError::Chain(format!("parse eve store: {e}")))?
        .ok_or_else(|| ChainError::Chain("launcher spend is not a datastore".into()))?;

    // Walk forward until the singleton coin is unspent (Live) or a spend yields
    // no child (Melted).
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
            return Ok(SingletonTerminal::Live(Box::new(store))); // current, unspent singleton
        }
        let spend = chain
            .coin_spend(coin_id, rec.spent_block_index)
            .await?
            .ok_or_else(|| ChainError::Chain("singleton spend not found".into()))?;
        let delegated: Vec<DelegatedPuzzle> = store.info.delegated_puzzles.clone();
        // Capture this generation's root + spend height BEFORE parsing the spend, so
        // a `Melted` classification can surface the last root and the burial-depth
        // anchor even though the melt spend produces no child to read them from.
        let last_root = store.info.metadata.root_hash;
        let melt_spent_height = rec.spent_block_index;
        // The melt signature (datalayer-driver 3.0.0): the terminal spend IS a valid
        // datastore singleton spend, but its inner conditions carry a MeltSingleton /
        // fee-reserve with NO odd (singleton) create-coin — so `from_spend` returns
        // `Err(DriverError::MissingChild)` (NOT `Ok(None)`, which the #1981 ticket
        // assumed). `MissingChild` is therefore the EXACT, and only, on-chain
        // signature of an owner-authorized melt. Every other error (unparseable
        // puzzle, CLVM failure, an intermediate that isn't a datastore at all) is a
        // corrupt/unexpected chain and MUST stay `Err` — corrupt is never melt.
        match DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &spend, &delegated) {
            // The spent singleton produced a datastore child: keep walking.
            Ok(Some(child)) => store = child,
            // The spend parsed but is not a datastore singleton at all — a corrupt /
            // unexpected terminal, NOT a melt. Preserves `sync_datastore`'s legacy
            // "singleton spend did not yield a store" error for this exact case.
            Ok(None) => {
                return Err(ChainError::Chain(
                    "singleton spend did not yield a store".into(),
                ));
            }
            // The one and only melt signature: a datastore singleton consumed with no
            // singleton child.
            Err(DriverError::MissingChild) => {
                return Ok(SingletonTerminal::Melted {
                    last_root,
                    melt_spent_height,
                });
            }
            // Any other parse error is a corrupt/unreadable terminal spend — stay Err.
            Err(e) => {
                return Err(ChainError::Chain(format!("parse next store: {e}")));
            }
        }
    }
}

/// Reconstructs the current unspent datastore singleton for `launcher_id` using
/// only coinset reads (coin records + puzzle/solution), following the singleton
/// lineage. No P2P peer required. Owner-only stores carry no delegated puzzles.
///
/// `DataStore::from_spend(ctx, spend, delegated)` returns the CHILD datastore
/// created by spending `spend.coin`, so we walk launcher -> eve -> ... forward
/// until we reach a singleton coin that is still unspent.
///
/// Behaviour is UNCHANGED from before the classifier was factored out: a
/// never-minted launcher or a melted lineage still returns [`ChainError::Chain`]
/// with the same message. Callers that need to DISTINGUISH those terminal states
/// (e.g. melt propagation) MUST use [`classify_singleton`] instead.
pub async fn sync_datastore(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<DataStore> {
    match walk_singleton_terminal(chain, launcher_id).await? {
        SingletonTerminal::Live(store) => Ok(*store),
        SingletonTerminal::NeverMinted => Err(ChainError::Chain(
            "launcher coin is unspent (store not minted yet)".into(),
        )),
        // Byte-identical to the pre-classifier behaviour: a melt surfaced through the
        // generic `from_spend` error mapping as "parse next store: missing child"
        // (melt == `Err(DriverError::MissingChild)`), so reproduce that exact string.
        SingletonTerminal::Melted { .. } => {
            Err(ChainError::Chain("parse next store: missing child".into()))
        }
    }
}

/// Classify the terminal state of the singleton lineage for `launcher_id`,
/// distinguishing a live store, a never-minted launcher, and a genuine owner melt
/// — the three states [`sync_datastore`] collapses into one error.
///
/// This is the canonical, custody-load-bearing classifier for the dig-node
/// store-melt-propagation path (#1316): a `Melted` result authorizes an
/// IRREVERSIBLE downstream data delete, so the semantics of each arm are exact —
///
/// - launcher UNSPENT ⇒ [`NeverMinted`](SingletonTerminal::NeverMinted);
/// - walk reaches an UNSPENT tip ⇒ [`Live`](SingletonTerminal::Live);
/// - a SPENT singleton coin whose spend yields no datastore child ⇒
///   [`Melted`](SingletonTerminal::Melted), carrying the last root and the melt
///   spend's block height (for burial-depth enforcement against reorgs);
/// - EVERY chain-read failure, missing/unparseable spend, or non-datastore
///   launcher spend ⇒ `Err`. **A corrupt or unreachable chain is NEVER classified
///   as `Melted`** — that is the single most important correctness property, since
///   a false `Melted` triggers permanent data loss downstream.
///
/// Shares its lineage traversal with [`sync_datastore`] (via
/// [`walk_singleton_terminal`]); there is exactly one walker.
pub async fn classify_singleton(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
) -> Result<SingletonTerminal> {
    walk_singleton_terminal(chain, launcher_id).await
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
        let mb = build_mint(&keys, &[coin], Bytes32::default(), None, None, 0).unwrap();
        // Now call build_update with an empty fee_coins slice and a nonzero fee.
        let result = build_update(
            &keys,
            mb.datastore,
            Bytes32::new([1u8; 32]),
            None,
            None,
            &[],
            1_000,
        );
        assert!(result.is_err(), "expected error with empty fee coins");
    }

    #[test]
    fn build_mint_produces_signed_bundle_and_launcher() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        // A synthetic funding coin at the owner puzzle hash (mint_store builds
        // the spend purely; it does not check on-chain existence).
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1_000_000);
        let mb = build_mint(&keys, &[coin], Bytes32::default(), None, None, 1_000).unwrap();
        assert!(!mb.bundle.coin_spends.is_empty());
        assert_ne!(mb.launcher_id, Bytes32::default()); // a real launcher id was derived
    }

    #[test]
    fn build_mint_embeds_label_and_description_in_metadata() {
        // The on-chain project name (label) + description must land in the singleton's
        // DataStoreMetadata, not be dropped — this is the write half of the feature.
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1_000_000);
        let mb = build_mint(
            &keys,
            &[coin],
            Bytes32::default(),
            Some("My Project".to_string()),
            Some("A test description".to_string()),
            1_000,
        )
        .unwrap();
        assert_eq!(
            mb.datastore.info.metadata.label.as_deref(),
            Some("My Project")
        );
        assert_eq!(
            mb.datastore.info.metadata.description.as_deref(),
            Some("A test description")
        );
    }

    #[test]
    fn build_mint_errors_when_insufficient_coins() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let coin = Coin::new(Bytes32::default(), keys.owner_puzzle_hash, 1); // < fee+1
        assert!(build_mint(&keys, &[coin], Bytes32::default(), None, None, 1_000).is_err());
    }

    /// #17 writer-key: a store minted with a writer's [`writer_delegated_puzzle`]
    /// can have its metadata root advanced by the WRITER key (not the owner), and
    /// the writer-authorized update VALIDATES on the in-process Chia simulator.
    /// This is the deploy-token primitive: a revocable delegate advances the root
    /// without the owner master seed. (No fee/DIG here — those are wallet-signed and
    /// concatenated by the caller; this proves the writer authorization itself.)
    #[test]
    fn writer_delegate_advances_root_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // Owner funds + mints a store that delegates writer authority to `writer`.
        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let writer = sim.bls(0); // the deploy-token key (no coins of its own)
        let writer_dp = writer_delegated_puzzle(writer.pk);

        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: Some("site".into()),
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![writer_dp],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        // The writer (NOT the owner) advances the root — no fee, no fee_keys needed.
        let new_root = Bytes32::new([0xab; 32]);
        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };
        let upd = build_update_unsigned_writer(
            writer.pk,
            store,
            new_root,
            Some("site".into()),
            None,
            &owner_keys,
            &[],
            0,
        )?;
        // The writer key alone signs the writer-authorized singleton spend (the
        // simulator validates against TESTNET11, so sign for testnet).
        let sig = sign_coin_spends(&upd.coin_spends, std::slice::from_ref(&writer.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(upd.coin_spends, sig))?;

        // The advanced singleton carries the new root.
        assert_eq!(upd.datastore.info.metadata.root_hash, new_root);
        Ok(())
    }

    /// #17: the writer authorization is REQUIRED — a store that does NOT delegate to
    /// the writer rejects a writer-authorized update (the puzzle is not in the set),
    /// so a stray key can never advance a store it was not granted.
    #[test]
    fn writer_update_rejected_without_delegation() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let writer = sim.bls(0);

        // Mint WITHOUT the writer's delegated puzzle.
        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };
        // Building the writer update against an un-delegated store fails (no writer
        // puzzle in the merkle set → the driver cannot construct the writer spend).
        let res = build_update_unsigned_writer(
            writer.pk,
            store,
            Bytes32::new([1u8; 32]),
            None,
            None,
            &owner_keys,
            &[],
            0,
        );
        assert!(
            res.is_err(),
            "writer update on an un-delegated store must fail"
        );
        Ok(())
    }

    /// The admin/writer/oracle delegated-puzzle derivations are STABLE and produce the
    /// expected `DelegatedPuzzle` variant — the byte-mirror of chip35's. A writer/admin
    /// for the same key are NOT interchangeable (distinct variants), and the oracle
    /// carries its fee + puzzle hash.
    #[test]
    fn delegated_puzzle_derivations_are_stable_and_typed() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let writer = writer_delegated_puzzle(keys.synthetic_pk);
        let admin = admin_delegated_puzzle(keys.synthetic_pk);
        let oracle = oracle_delegated_puzzle(keys.owner_puzzle_hash, 1000);
        assert!(matches!(writer, DelegatedPuzzle::Writer(_)));
        assert!(matches!(admin, DelegatedPuzzle::Admin(_)));
        assert!(
            matches!(oracle, DelegatedPuzzle::Oracle(ph, fee) if ph == keys.owner_puzzle_hash && fee == 1000)
        );
        // Deterministic: the same key derives the same delegate twice.
        assert_eq!(admin, admin_delegated_puzzle(keys.synthetic_pk));
        // Writer and admin for the same key are distinct authorities.
        assert_ne!(
            format!("{writer:?}"),
            format!("{admin:?}"),
            "writer and admin delegates must not collide"
        );
    }

    /// An owner can MELT their store on the simulator — the singleton spend validates
    /// and consumes the singleton (build via `melt_store`, no DIG/fee coin needed).
    #[test]
    fn owner_melts_store_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };
        let melt = build_melt_unsigned(&owner_keys, store.clone())?;
        let sig = sign_coin_spends(&melt.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(melt.coin_spends, sig))?;

        // The singleton coin is now spent (melted): there is no unspent child.
        assert!(
            sim.coin_state(store.coin.coin_id())
                .is_some_and(|cs| cs.spent_height.is_some()),
            "melted singleton coin must be spent"
        );
        Ok(())
    }

    /// An owner can DELEGATE writer authority to a new key via an ownership update
    /// (`update_store_ownership`, owner unchanged, delegated set grows), and that
    /// newly-delegated writer can then advance the root — proving delegation wires the
    /// on-chain authorization end-to-end. This is the Teams "add a Deployer" flow.
    #[test]
    fn owner_delegates_writer_then_writer_advances_root() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // Mint a store with NO delegates.
        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let writer = sim.bls(0);
        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: Some("site".into()),
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };

        // Owner delegates writer authority (owner unchanged, delegated set += writer).
        let writer_dp = writer_delegated_puzzle(writer.pk);
        let upd = build_update_ownership_unsigned(
            &owner_keys,
            store,
            owner.puzzle_hash, // ownership unchanged
            vec![writer_dp],   // NEW delegated set
            &[],
            0,
        )?;
        let sig = sign_coin_spends(&upd.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(upd.coin_spends, sig))?;
        let delegated_store = upd.datastore;

        // The newly-delegated writer can now advance the root.
        let new_root = Bytes32::new([0xcd; 32]);
        let wupd = build_update_unsigned_writer(
            writer.pk,
            delegated_store,
            new_root,
            Some("site".into()),
            None,
            &owner_keys,
            &[],
            0,
        )?;
        let wsig = sign_coin_spends(&wupd.coin_spends, std::slice::from_ref(&writer.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(wupd.coin_spends, wsig))?;
        assert_eq!(wupd.datastore.info.metadata.root_hash, new_root);
        Ok(())
    }

    /// Delegating a SECOND writer must NOT revoke the first (#24 `authorize-origin-as-writer`
    /// relies on this: it reads the CURRENT delegated set and re-sends it plus the new writer,
    /// since `new_delegated_puzzles` REPLACES the whole set at the driver level). Proves on a
    /// real simulator that after two sequential delegate-adds, BOTH writers can independently
    /// advance the root.
    #[test]
    fn owner_delegates_second_writer_without_revoking_the_first() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let writer_a = sim.bls(0);
        let writer_b = sim.bls(0);
        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: Some("site".into()),
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };

        // Delegate writer A (starting from an empty delegated set).
        let writer_a_dp = writer_delegated_puzzle(writer_a.pk);
        let upd_a = build_update_ownership_unsigned(
            &owner_keys,
            store,
            owner.puzzle_hash,
            vec![writer_a_dp],
            &[],
            0,
        )?;
        let sig_a = sign_coin_spends(&upd_a.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(upd_a.coin_spends, sig_a))?;

        // Delegate writer B by re-sending the CURRENT delegated set (read back from the store
        // returned by the first update) plus the new delegate — exactly the pattern
        // `authorize-origin-as-writer` follows.
        let mut delegates = upd_a.datastore.info.delegated_puzzles.clone();
        assert_eq!(
            delegates,
            vec![writer_a_dp],
            "writer A must be present before adding B"
        );
        let writer_b_dp = writer_delegated_puzzle(writer_b.pk);
        delegates.push(writer_b_dp);
        let upd_b = build_update_ownership_unsigned(
            &owner_keys,
            upd_a.datastore,
            owner.puzzle_hash,
            delegates,
            &[],
            0,
        )?;
        let sig_b = sign_coin_spends(&upd_b.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(upd_b.coin_spends, sig_b))?;

        let both_delegated_store = upd_b.datastore;
        assert_eq!(
            both_delegated_store.info.delegated_puzzles.len(),
            2,
            "both writer A and writer B must be delegated"
        );
        assert!(both_delegated_store
            .info
            .delegated_puzzles
            .contains(&writer_a_dp));
        assert!(both_delegated_store
            .info
            .delegated_puzzles
            .contains(&writer_b_dp));

        // Writer A (delegated FIRST) can still advance the root after B was added.
        let root_after_a = Bytes32::new([0xaa; 32]);
        let wupd_a = build_update_unsigned_writer(
            writer_a.pk,
            both_delegated_store,
            root_after_a,
            Some("site".into()),
            None,
            &owner_keys,
            &[],
            0,
        )?;
        let wsig_a = sign_coin_spends(
            &wupd_a.coin_spends,
            std::slice::from_ref(&writer_a.sk),
            true,
        )
        .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(wupd_a.coin_spends, wsig_a))?;
        assert_eq!(wupd_a.datastore.info.metadata.root_hash, root_after_a);

        // Writer B can also independently advance the root.
        let root_after_b = Bytes32::new([0xbb; 32]);
        let wupd_b = build_update_unsigned_writer(
            writer_b.pk,
            wupd_a.datastore,
            root_after_b,
            Some("site".into()),
            None,
            &owner_keys,
            &[],
            0,
        )?;
        let wsig_b = sign_coin_spends(
            &wupd_b.coin_spends,
            std::slice::from_ref(&writer_b.sk),
            true,
        )
        .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(wupd_b.coin_spends, wsig_b))?;
        assert_eq!(wupd_b.datastore.info.metadata.root_hash, root_after_b);

        Ok(())
    }

    /// An owner can TRANSFER the store to a new owner via an ownership update
    /// (`update_store_ownership`, `new_owner_ph` = recipient). The recreated store
    /// carries the recipient's owner puzzle hash, and the spend validates on the
    /// simulator. This is `chia_setStoreOwnership`.
    #[test]
    fn owner_transfers_store_to_new_owner_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        let owner = sim.bls(2);
        let owner_p2 = StandardLayer::new(owner.pk);
        let recipient = sim.bls(0);
        let (launch, store) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: Bytes32::default(),
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;

        let owner_keys = WalletKeys {
            synthetic_sk: owner.sk.clone(),
            synthetic_pk: owner.pk,
            owner_puzzle_hash: owner.puzzle_hash,
        };
        let xfer = build_update_ownership_unsigned(
            &owner_keys,
            store,
            recipient.puzzle_hash, // transfer to recipient
            vec![],
            &[],
            0,
        )?;
        let sig = sign_coin_spends(&xfer.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
        sim.new_transaction(SpendBundle::new(xfer.coin_spends, sig))?;
        assert_eq!(
            xfer.datastore.info.owner_puzzle_hash, recipient.puzzle_hash,
            "transferred store must carry the recipient's owner puzzle hash"
        );
        Ok(())
    }
}

/// Read the launcher's current on-chain root by syncing its singleton lineage
/// over `chain` and returning the latest metadata root. Errors (propagated from
/// `sync_datastore`) mean the chain could not be read — callers MUST fail closed.
pub async fn current_root(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<Bytes32> {
    let store = sync_datastore(chain, launcher_id).await?;
    Ok(store.info.metadata.root_hash)
}

/// Verify — **fail-closed**, without a full forward lineage walk — that `pinned_root` is
/// the store's CURRENT on-chain generation.
///
/// A local content read of `dig://<store_id>:<pinned_root>` (the §5.3 loopback tier) MUST
/// NOT serve bytes for a root it cannot chain-anchor. The obvious anchor —
/// [`current_root`] / [`sync_datastore`] — walks the singleton lineage launcher → eve →
/// … → tip FORWARD, parsing every generation's SPEND. A store with hundreds of generations
/// makes that walk expensive, and a SINGLE unparseable intermediate spend aborts the whole
/// walk (issue #747: "parse next store: missing child"), so even a perfectly valid pinned
/// root becomes unreadable. This verify avoids that forward walk.
///
/// # Identity is anchored on the launcher coin — NEVER the curried `launcher_id`
/// The current unspent singleton is discovered with one
/// [`unspent_coins_by_hint`](ChainReads::unspent_coins_by_hint) query on `store_id`. A hint
/// is an attacker-controllable CREATE_COIN memo, and `DataStore::from_spend` reads
/// `launcher_id` STRAIGHT from a spend's curried `SingletonLayer` — Chia's singleton
/// top-layer does not bind that curried value to the real launcher coin. So a coin merely
/// discovered by hint whose curried `launcher_id == store_id` is NOT proof of identity: an
/// attacker can forge exactly that (issue #1473). The only unforgeable identity is the
/// launcher coin itself — `coin_id == store_id` is a 256-bit hash preimage an attacker
/// cannot grind. So each candidate tip is accepted only when it PROVABLY DESCENDS from that
/// launcher, via a bounded BACKWARD walk of COIN RECORDS (following `coin.parent_coin_info`,
/// capped at `MAX_HOPS`) that must reach the coin whose `coin_id == store_id` — and that
/// launcher coin must itself exist, be spent, and carry the singleton launcher puzzle hash.
///
/// #747-immunity is preserved because the backward walk needs only COIN RECORDS, never the
/// intermediate generations' SPENDS. The tip's root is still read from the ONE spend that
/// created it (the tip's parent). As defense-in-depth, each hop whose spend IS available is
/// checked to curry `launcher_id == store_id` — best-effort only: a missing/unparseable
/// intermediate spend never fails the verification (the coin-record chain is the real proof).
///
/// # Anti-rollback (§5.3 / NC-9)
/// This is the strongest anti-rollback stance: only the CURRENT on-chain root of the store
/// whose launcher is `store_id` passes. A root that was never on-chain, a STALE (superseded)
/// generation, or a forged singleton not descending from the launcher coin is rejected.
///
/// # Errors
/// Returns `Err` — never a false `Ok` — whenever the root cannot be positively
/// chain-anchored: the chain is unreachable, no launcher-anchored unspent singleton for
/// `store_id` is found, or the on-chain current root differs from `pinned_root`. Callers
/// MUST treat any `Err` as "do not serve".
///
/// `store_id` is the store's `launcher_id`.
pub async fn verify_pinned_root(
    chain: &dyn ChainReads,
    store_id: Bytes32,
    pinned_root: Bytes32,
) -> Result<()> {
    // The current unspent singleton is hinted to launcher_id (== store_id): fetch it
    // directly, no forward walk. A hint is attacker-controllable, so every candidate is
    // anchored to the unforgeable launcher coin below before its root is trusted.
    let candidates = chain.unspent_coins_by_hint(store_id).await?;

    let mut ctx = SpendContext::new();
    for coin in candidates {
        // Read the candidate tip's on-chain root from the ONE spend that created it — the
        // spend of its parent. This is the only generation whose spend we require; earlier
        // generations (which may be unparseable, #747) are reached only via coin records.
        let parent_id = coin.parent_coin_info;
        let Some(parent_rec) = chain.coin_record(parent_id).await? else {
            continue;
        };
        if !parent_rec.spent {
            continue;
        }
        let Some(parent_spend) = chain
            .coin_spend(parent_id, parent_rec.spent_block_index)
            .await?
        else {
            continue;
        };
        // The parent's own creation memos carry its delegated-puzzle set, so from_spend
        // reconstructs the child without a caller-supplied delegated list (`&[]`).
        let Ok(Some(store)) =
            DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &parent_spend, &[])
        else {
            continue;
        };
        // Best-effort defense-in-depth on the tip's parent spend: its curried launcher_id
        // must equal store_id. (This is NOT the identity proof — the backward walk is.)
        if store.info.launcher_id != store_id {
            continue; // a different store sharing the hint, or a mis-curried forgery
        }

        // THE SOUNDNESS ANCHOR: prove this tip descends from the launcher coin whose
        // coin_id == store_id. Without this, a forged singleton currying store_id (issue
        // #1473) would be trusted. A candidate that cannot be anchored is skipped.
        if !tip_descends_from_launcher(chain, coin, store_id).await? {
            continue;
        }

        // This IS the current unspent generation of the launcher-anchored `store_id`.
        // Fail-closed: the pinned root must equal the live on-chain root exactly.
        if store.info.metadata.root_hash == pinned_root {
            return Ok(());
        }
        return Err(ChainError::Chain(format!(
            "pinned root {pinned_root:?} is not the current on-chain root {:?} for store {store_id:?}",
            store.info.metadata.root_hash
        )));
    }

    Err(ChainError::Chain(format!(
        "no current unspent singleton found on chain for store {store_id:?}; pinned root unverifiable"
    )))
}

/// Prove `tip` descends from the launcher coin whose `coin_id == store_id`, using ONLY
/// coin records (never intermediate spends, preserving #747-immunity).
///
/// Walks `coin.parent_coin_info` upward from `tip`, bounded by `MAX_HOPS`, until a hop's
/// parent is `store_id`; then confirms that launcher coin exists, is spent, and carries the
/// singleton launcher puzzle hash. As best-effort defense-in-depth, each ancestor hop whose
/// spend is available and parseable must curry `launcher_id == store_id`; a missing or
/// unparseable spend is skipped (the coin-record parent chain is the real proof).
///
/// Returns `Ok(true)` only when the tip is provably launcher-anchored; `Ok(false)` for any
/// unanchored candidate (missing ancestor, hop limit, bad launcher, or a parseable spend
/// currying a different launcher_id). Chain-read failures propagate as `Err`.
async fn tip_descends_from_launcher(
    chain: &dyn ChainReads,
    tip: Coin,
    store_id: Bytes32,
) -> Result<bool> {
    const MAX_HOPS: u32 = 100_000; // mirror sync_datastore's bound

    let mut ctx = SpendContext::new();
    let mut current = tip;
    let mut hops = 0u32;
    loop {
        hops += 1;
        if hops > MAX_HOPS {
            return Ok(false); // cycle or corrupt chain data — fail closed
        }
        let parent_id = current.parent_coin_info;
        if parent_id == store_id {
            // Reached the launcher. Confirm it is a real, spent singleton launcher coin.
            return verify_launcher_coin(chain, store_id).await;
        }
        let Some(parent_rec) = chain.coin_record(parent_id).await? else {
            return Ok(false); // an ancestor is missing — cannot anchor
        };
        // Best-effort per-hop check: if the ancestor's spend is available and parses as a
        // datastore, its curried launcher_id must match store_id. A missing/unparseable
        // spend is intentionally skipped (that is the whole reason this API exists, #747).
        if parent_rec.spent {
            if let Ok(Some(spend)) = chain
                .coin_spend(parent_id, parent_rec.spent_block_index)
                .await
            {
                if let Ok(Some(hop_store)) =
                    DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &spend, &[])
                {
                    if hop_store.info.launcher_id != store_id {
                        return Ok(false); // a parseable hop that curries a different store
                    }
                }
            }
        }
        current = parent_rec.coin;
    }
}

/// Confirm the coin whose `coin_id == store_id` is a real singleton launcher: it exists, is
/// spent (a launcher is spent to create the eve singleton), and carries the well-known
/// singleton launcher puzzle hash. A `store_id` that is not a launched singleton's launcher
/// must fail closed.
async fn verify_launcher_coin(chain: &dyn ChainReads, store_id: Bytes32) -> Result<bool> {
    let Some(launcher) = chain.coin_record(store_id).await? else {
        return Ok(false);
    };
    Ok(launcher.spent && launcher.coin.puzzle_hash == SINGLETON_LAUNCHER_PH)
}

// ===========================================================================
// Store discovery — a user's own DataLayer stores → their capsules.
//
// A store's launcher coin is created carrying `digstore_owner_hint(owner_ph)` as
// its indexed memo (see `mint_store_digstore`), IDENTICAL to chip35. So a single
// coinset `get_coin_records_by_hint` query for that hint surfaces every store a
// given owner address launched — whether minted by the CLI or the web app. The
// launcher coin's `coin_id` IS the `launcher_id` IS the `store_id`.
// ===========================================================================

/// Convert a `chia_protocol::Bytes32` into the ecosystem-canonical
/// `digstore_core::Bytes32` (both are raw 32-byte wrappers). Used to speak the one
/// shared [`Capsule`](digstore_core::Capsule) identity in discovery return types.
fn core_bytes32(b: Bytes32) -> digstore_core::Bytes32 {
    digstore_core::Bytes32(b.to_bytes())
}

/// Enumerate the launcher (store) ids of every DataLayer store owned by
/// `owner_ph`, by querying coinset for launcher coins carrying that owner's
/// discovery hint.
///
/// The hint is `digstore_owner_hint(owner_ph)` — the SAME derivation used at mint
/// time and by chip35 (hub.dig.net), so one query finds CLI- and web-minted stores
/// alike. Each returned [`Bytes32`] is a `coin_id` which, for a launcher coin,
/// equals the `launcher_id` and therefore the `store_id`.
///
/// Returns the store ids of currently-unspent launcher coins (a launcher coin is
/// spent exactly once — to create the eve singleton — but `unspent_coins_by_hint`
/// keys on the hint that the singleton lineage continues to carry; in practice a
/// live store's discoverable coin is its current singleton, hinted to the owner).
pub async fn enum_user_stores(chain: &dyn ChainReads, owner_ph: Bytes32) -> Result<Vec<Bytes32>> {
    let hint = digstore_owner_hint(owner_ph);
    let coins = chain.unspent_coins_by_hint(hint).await?;
    Ok(coins.into_iter().map(|c| c.coin_id()).collect())
}

/// Discover ALL of a wallet's DataLayer stores across its HD addresses.
///
/// Derives the wallet's unhardened keys for indices `0..500` (the same gap-limit
/// the [`scan_wallet`](crate::wallet::scan_wallet) scan uses) and runs
/// [`enum_user_stores`] for each address's owner puzzle hash, returning
/// `(hd_index, store_id)` pairs so the caller knows which derived address owns each
/// store. A wallet that reused one address has all its stores under index 0; a
/// wallet that rotated addresses has them spread across indices.
pub async fn discover_all_user_stores(
    chain: &dyn ChainReads,
    mnemonic: &str,
) -> Result<Vec<(u32, Bytes32)>> {
    let indexed = crate::keys::derive_indexed_keys(mnemonic, 0..500)?;
    let mut out = Vec::new();
    for k in indexed {
        let stores = enum_user_stores(chain, k.owner_puzzle_hash).await?;
        for store_id in stores {
            out.push((k.index, store_id));
        }
    }
    Ok(out)
}

/// The current state of a store plus its ordered capsule (root) history.
///
/// `current` is the live capsule — `(launcher_id, current_root)` — and `history`
/// is the ordered list of every capsule the store has been, one
/// [`Capsule`](digstore_core::Capsule) per on-chain root from the eve singleton
/// (index 0) through to the current one (the last element of `history`, which has
/// the same root as `current`). Each commit adds one capsule.
pub struct StoreHistory {
    /// The store's CURRENT capsule: `(launcher_id, current_root)`.
    pub current: digstore_core::Capsule,
    /// Every capsule the store has ever been, oldest → newest. The last element's
    /// `root_hash` equals `current.root_hash`.
    pub history: Vec<digstore_core::Capsule>,
}

/// Sync a store AND collect its full capsule (root) history during the walk.
///
/// This is [`sync_datastore`]'s forward walk, except every hop's metadata root is
/// COLLECTED into the ordered capsule history instead of discarded. The walk goes
/// eve → … → current unspent singleton; each visited singleton's root becomes one
/// [`Capsule`](digstore_core::Capsule) `(launcher_id, root)`. The returned
/// `current` capsule is `(launcher_id, current_root)` (== the last history entry),
/// and the returned [`DataStore`] is the live, spendable singleton.
///
/// Returns `(DataStore, StoreHistory)` so callers get both the spendable handle
/// (for further updates) and the audit-grade capsule lineage.
pub async fn sync_datastore_with_history(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
) -> Result<(DataStore, StoreHistory)> {
    let mut ctx = SpendContext::new();
    let store_id = core_bytes32(launcher_id);

    // The launcher coin is spent to create the eve singleton (same as sync_datastore).
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

    // Collect each hop's root as a capsule, in order.
    let mut history: Vec<digstore_core::Capsule> = Vec::new();
    let mut push_capsule = |store: &DataStore| {
        history.push(digstore_core::Capsule {
            store_id,
            root_hash: core_bytes32(store.info.metadata.root_hash),
        });
    };

    const MAX_HOPS: u32 = 100_000;
    let mut hops = 0u32;
    loop {
        hops += 1;
        if hops > MAX_HOPS {
            return Err(ChainError::Chain(format!(
                "singleton chain exceeded {MAX_HOPS} hops; possible cycle or corrupt chain data"
            )));
        }
        // Record this generation's capsule (root) before checking if it is the tip.
        push_capsule(&store);

        let coin_id = store.coin.coin_id();
        let rec = chain
            .coin_record(coin_id)
            .await?
            .ok_or_else(|| ChainError::Chain(format!("singleton coin {coin_id:?} not found")))?;
        if !rec.spent {
            // `store` is the current, unspent singleton; the last-pushed capsule is current.
            let current = *history
                .last()
                .expect("at least one capsule pushed before the unspent check");
            return Ok((store, StoreHistory { current, history }));
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
        let built = build_update(&keys, store, new_root, None, None, &fee_coins, 1_000).unwrap();
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

// ===========================================================================
// Store-discovery tests — enum_user_stores / discover_all_user_stores /
// sync_datastore_with_history, all on the in-crate MockChain (no network).
// ===========================================================================
#[cfg(test)]
mod discovery_tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::{CoinInfo, CoinRecord};
    use crate::keys::derive_indexed_keys;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// Seed one unspent launcher coin under `owner_ph`'s discovery hint, returning its
    /// store id (= coin id).
    ///
    /// The launcher coin lives at the well-known singleton launcher puzzle hash and
    /// carries 1 mojo (odd amount) — exactly how `mint_store` creates it — so its
    /// `coin_id` is a faithful stand-in for a real launcher id (= store id).
    fn seed_store(mock: &mut MockChain, owner_ph: Bytes32, parent: [u8; 32]) -> Bytes32 {
        let coin = Coin::new(Bytes32::new(parent), SINGLETON_LAUNCHER_PH, 1);
        let store_id = coin.coin_id();
        let hint = digstore_owner_hint(owner_ph);
        mock.records_by_hint
            .entry(hint)
            .or_default()
            .push(CoinRecord {
                coin,
                spent: false,
                confirmed_block_index: 100,
                spent_block_index: 0,
                timestamp: 1_700_000_000,
                coinbase: false,
            });
        store_id
    }

    // enum_user_stores: a single owner's hint-indexed launcher surfaces as a store id.
    #[tokio::test]
    async fn enum_user_stores_finds_hinted_launcher() {
        let mut mock = MockChain::default();
        let keys = derive_indexed_keys(ABANDON, 0..1).unwrap();
        let owner_ph = keys[0].owner_puzzle_hash;
        let store_id = seed_store(&mut mock, owner_ph, [1u8; 32]);

        let found = enum_user_stores(&mock, owner_ph).await.unwrap();
        assert_eq!(
            found,
            vec![store_id],
            "the launcher's coin_id is the store id"
        );
    }

    // enum_user_stores: an owner with no stores returns empty (and only that owner's
    // hint is consulted — a different owner's store does not leak in).
    #[tokio::test]
    async fn enum_user_stores_empty_for_owner_without_stores() {
        let mut mock = MockChain::default();
        let keys = derive_indexed_keys(ABANDON, 0..2).unwrap();
        // Seed a store for index 1 only.
        seed_store(&mut mock, keys[1].owner_puzzle_hash, [2u8; 32]);

        let found = enum_user_stores(&mock, keys[0].owner_puzzle_hash)
            .await
            .unwrap();
        assert!(found.is_empty(), "index 0 owns no stores");
    }

    // discover_all_user_stores: stores spread across HD indices are all found, each
    // tagged with the HD index of the address that owns it.
    #[tokio::test]
    async fn discover_all_user_stores_spans_hd_indices() {
        let mut mock = MockChain::default();
        let keys = derive_indexed_keys(ABANDON, 0..5).unwrap();
        // One store at index 0, two at index 3.
        let s0 = seed_store(&mut mock, keys[0].owner_puzzle_hash, [10u8; 32]);
        let s3a = seed_store(&mut mock, keys[3].owner_puzzle_hash, [30u8; 32]);
        let s3b = seed_store(&mut mock, keys[3].owner_puzzle_hash, [31u8; 32]);

        let mut found = discover_all_user_stores(&mock, ABANDON).await.unwrap();
        found.sort_by_key(|(idx, _)| *idx);

        assert_eq!(found.len(), 3, "all three stores discovered");
        assert!(found.contains(&(0u32, s0)));
        assert!(found.contains(&(3u32, s3a)));
        assert!(found.contains(&(3u32, s3b)));
    }

    // sync_datastore_with_history: a multi-generation lineage yields the ORDERED
    // capsule history (one capsule per root) and a current capsule == the last entry.
    #[tokio::test]
    async fn sync_with_history_collects_ordered_capsule_roots() {
        use crate::keys::derive_wallet_keys;

        // Build a REAL lineage with the driver: mint (root r0) then two updates
        // (r1, r2), seeding the MockChain from the actual coin spends so the walk
        // re-parses genuine singleton spends.
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let r0 = Bytes32::default();
        let r1 = Bytes32::new([1u8; 32]);
        let r2 = Bytes32::new([2u8; 32]);

        let funding = Coin::new(Bytes32::new([9u8; 32]), keys.owner_puzzle_hash, 1_000_000);
        let mint = build_mint(&keys, &[funding], r0, None, None, 0).unwrap();
        let launcher_id = mint.launcher_id;

        let mut mock = MockChain::default();

        // Helper: register a coin as spent + its spend (look the spend up by the
        // coin it spends).
        fn register_spend(mock: &mut MockChain, spends: &[CoinSpend], coin_id: Bytes32) {
            let cs = spends
                .iter()
                .find(|cs| cs.coin.coin_id() == coin_id)
                .expect("coin spend present in bundle")
                .clone();
            mock.records.insert(
                coin_id,
                CoinInfo {
                    coin: cs.coin,
                    spent: true,
                    confirmed_block_index: 100,
                    spent_block_index: 200,
                    timestamp: 1_700_000_000,
                    coinbase: false,
                },
            );
            mock.spends.insert(coin_id, cs);
        }

        // Advance the store's root with no fee (the multi variant skips fee-coin
        // selection when fee == 0, so the test needs no extra funding coin).
        let update_no_fee = |store: DataStore, new_root: Bytes32| {
            build_update_unsigned_multi(keys.synthetic_pk, store, new_root, None, None, &[], 0)
                .unwrap()
        };

        // The launcher coin is spent in the mint bundle to create the eve singleton.
        register_spend(&mut mock, &mint.bundle.coin_spends, launcher_id);

        // Generation 0 (eve): spent by the first update.
        let eve = mint.datastore.clone();
        let up1 = update_no_fee(eve.clone(), r1);
        register_spend(&mut mock, &up1.coin_spends, eve.coin.coin_id());

        // Generation 1: spent by the second update.
        let gen1 = up1.datastore.clone();
        let up2 = update_no_fee(gen1.clone(), r2);
        register_spend(&mut mock, &up2.coin_spends, gen1.coin.coin_id());

        // Generation 2: the current, UNSPENT tip.
        let gen2 = up2.datastore.clone();
        mock.records.insert(
            gen2.coin.coin_id(),
            CoinInfo {
                coin: gen2.coin,
                spent: false,
                confirmed_block_index: 300,
                spent_block_index: 0,
                timestamp: 1_700_000_001,
                coinbase: false,
            },
        );

        let (store, hist) = sync_datastore_with_history(&mock, launcher_id)
            .await
            .unwrap();

        // Current store is the unspent tip carrying r2.
        assert_eq!(store.info.metadata.root_hash, r2);
        assert_eq!(store.coin.coin_id(), gen2.coin.coin_id());

        // History is the ordered capsule list r0 → r1 → r2, all under the launcher id.
        let store_id = core_bytes32(launcher_id);
        let roots: Vec<digstore_core::Bytes32> = hist.history.iter().map(|c| c.root_hash).collect();
        assert_eq!(
            roots,
            vec![core_bytes32(r0), core_bytes32(r1), core_bytes32(r2)],
            "capsule history must be the ordered root list (eve → … → current)"
        );
        assert!(hist.history.iter().all(|c| c.store_id == store_id));

        // Current capsule == last history entry == (launcher_id, r2).
        assert_eq!(
            hist.current,
            digstore_core::Capsule {
                store_id,
                root_hash: core_bytes32(r2)
            }
        );
        assert_eq!(hist.current, *hist.history.last().unwrap());
    }
}

// ===========================================================================
// verify_pinned_root — the #747 / #852 regression suite.
//
// Builds a REAL multi-generation datastore lineage on the in-process Chia
// simulator, then feeds coinset-shaped reads into the in-crate MockChain. The
// central regression (`pinned_root_verifies_when_full_walk_is_broken`) proves
// that `verify_pinned_root` chain-anchors the current root even when an EARLIER
// generation's spend is unreadable — exactly the condition under which
// `sync_datastore`'s full walk aborts with "singleton spend not found" (#747),
// which the same fixture asserts.
// ===========================================================================
#[cfg(test)]
mod verify_pinned_root_tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::CoinInfo;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

    /// A three-generation store built on `sim`: eve(root0) → gen1(root1) → gen2(root2, tip).
    struct Lineage {
        store_id: Bytes32,
        /// (coin_id, is_current_tip) for launcher, eve, gen1, gen2 — everything the
        /// MockChain must know about to answer the reads.
        eve_coin: Bytes32,
        gen1_coin: Bytes32,
        gen2_coin: Bytes32,
        root0: Bytes32,
        root1: Bytes32,
        root2: Bytes32,
    }

    /// Mint an owner-only store and advance its root twice, applying every spend on
    /// `sim`. Returns the lineage's coin ids + roots so a MockChain can be seeded.
    fn build_three_gen_store(sim: &mut Simulator) -> anyhow::Result<Lineage> {
        let owner = sim.bls(3);
        let owner_p2 = StandardLayer::new(owner.pk);
        let ctx = &mut SpendContext::new();

        let root0 = Bytes32::new([0x10; 32]);
        let (launch, eve) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: root0,
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;
        let store_id = eve.info.launcher_id;

        // gen1: advance root0 → root1 (owner-authorized).
        let root1 = Bytes32::new([0x20; 32]);
        let up1 = update_store_metadata(
            eve.clone(),
            root1,
            None,
            None,
            None,
            None,
            DataStoreInnerSpend::Owner(owner.pk),
        )
        .map_err(|e| anyhow::anyhow!("update1: {e}"))?;
        let sig1 = sign_coin_spends(&up1.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign1: {e}"))?;
        sim.new_transaction(SpendBundle::new(up1.coin_spends, sig1))?;
        let gen1 = up1.new_datastore;

        // gen2 (the tip): advance root1 → root2.
        let root2 = Bytes32::new([0x30; 32]);
        let up2 = update_store_metadata(
            gen1.clone(),
            root2,
            None,
            None,
            None,
            None,
            DataStoreInnerSpend::Owner(owner.pk),
        )
        .map_err(|e| anyhow::anyhow!("update2: {e}"))?;
        let sig2 = sign_coin_spends(&up2.coin_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign2: {e}"))?;
        sim.new_transaction(SpendBundle::new(up2.coin_spends, sig2))?;
        let gen2 = up2.new_datastore;

        Ok(Lineage {
            store_id,
            eve_coin: eve.coin.coin_id(),
            gen1_coin: gen1.coin.coin_id(),
            gen2_coin: gen2.coin.coin_id(),
            root0,
            root1,
            root2,
        })
    }

    /// Seed a MockChain with a `coin_record` (+ `coin_spend` for spent coins) per id,
    /// pulled from the simulator — mirroring `mock_from_sim`.
    fn seed_records(sim: &Simulator, mock: &mut MockChain, coin_ids: &[Bytes32]) {
        for &coin_id in coin_ids {
            let Some(cs) = sim.coin_state(coin_id) else {
                continue;
            };
            let spent_h = cs.spent_height;
            mock.records.insert(
                coin_id,
                CoinInfo {
                    coin: cs.coin,
                    spent: spent_h.is_some(),
                    confirmed_block_index: cs.created_height.unwrap_or(0),
                    spent_block_index: spent_h.unwrap_or(0),
                    timestamp: 0,
                    coinbase: false,
                },
            );
            if spent_h.is_some() {
                if let Some(spend) = sim.coin_spend(coin_id) {
                    mock.spends.insert(coin_id, spend);
                }
            }
        }
    }

    /// Build a MockChain seeded to answer `verify_pinned_root`: records for the whole
    /// lineage plus a hint index mapping `store_id` → the unspent tip (gen2), exactly
    /// as coinset returns for a datastore's launcher-id hint.
    fn seed_mock(sim: &Simulator, lin: &Lineage) -> MockChain {
        let mut mock = MockChain::default();
        seed_records(
            sim,
            &mut mock,
            &[lin.store_id, lin.eve_coin, lin.gen1_coin, lin.gen2_coin],
        );
        let tip_rec = mock
            .records
            .get(&lin.gen2_coin)
            .expect("tip record seeded")
            .clone();
        mock.records_by_hint.insert(lin.store_id, vec![tip_rec]);
        mock
    }

    /// GREEN: the pinned CURRENT root of a multi-generation store verifies without a walk.
    #[tokio::test]
    async fn pinned_current_root_verifies() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let lin = build_three_gen_store(&mut sim)?;
        let mock = seed_mock(&sim, &lin);
        verify_pinned_root(&mock, lin.store_id, lin.root2)
            .await
            .expect("current root chain-anchors");
        Ok(())
    }

    /// THE #747 REGRESSION: with an EARLIER generation's spend unreadable, the full
    /// lineage walk (`sync_datastore`) aborts — yet `verify_pinned_root` still
    /// chain-anchors the current root, because it only parses the tip's parent spend.
    #[tokio::test]
    async fn pinned_root_verifies_when_full_walk_is_broken() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let lin = build_three_gen_store(&mut sim)?;
        let mut mock = seed_mock(&sim, &lin);

        // Simulate #747: the eve→gen1 spend is unreadable. The forward walk needs it and
        // cannot proceed past eve.
        mock.spends.remove(&lin.eve_coin);

        let walk_err = sync_datastore(&mock, lin.store_id).await.unwrap_err();
        match walk_err {
            ChainError::Chain(msg) => assert!(
                msg.contains("singleton spend not found"),
                "full walk should break on the missing intermediate spend; got: {msg}"
            ),
            other => panic!("expected Chain error from the broken walk, got {other:?}"),
        }

        // The pinned-root verify sidesteps the break entirely.
        verify_pinned_root(&mock, lin.store_id, lin.root2)
            .await
            .expect("pinned current root verifies despite the broken lineage walk");
        Ok(())
    }

    /// FAIL-CLOSED: a STALE (superseded) generation's root is rejected — anti-rollback.
    #[tokio::test]
    async fn stale_root_is_rejected() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let lin = build_three_gen_store(&mut sim)?;
        let mock = seed_mock(&sim, &lin);
        for stale in [lin.root0, lin.root1] {
            let err = verify_pinned_root(&mock, lin.store_id, stale)
                .await
                .expect_err("a superseded root must not verify");
            match err {
                ChainError::Chain(msg) => {
                    assert!(msg.contains("not the current on-chain root"), "got: {msg}")
                }
                other => panic!("expected root-mismatch Chain error, got {other:?}"),
            }
        }
        Ok(())
    }

    /// FAIL-CLOSED: a root that was NEVER on-chain is rejected.
    #[tokio::test]
    async fn never_seen_root_is_rejected() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let lin = build_three_gen_store(&mut sim)?;
        let mock = seed_mock(&sim, &lin);
        let err = verify_pinned_root(&mock, lin.store_id, Bytes32::new([0xff; 32]))
            .await
            .expect_err("a fabricated root must not verify");
        assert!(matches!(err, ChainError::Chain(_)));
        Ok(())
    }

    /// SECURITY (issue #1473): a forged singleton that curries `launcher_id == store_id`
    /// but does NOT descend from the launcher coin (coin_id == store_id) MUST be rejected.
    ///
    /// `DataStore::from_spend` reads `launcher_id` straight from the parent's curried
    /// `SingletonLayer`, and Chia's singleton top-layer does not bind that curried value to
    /// the real launcher coin. So an attacker can construct a spend that from_spends to
    /// `launcher_id == store_id` + `root == R_evil` from a coin whose id != store_id, hint
    /// it to `store_id`, and — under the pre-#1473 verify, which gated ONLY on the curried
    /// `launcher_id` — get an honest node to serve `R_evil` as chain-verified. The fixed
    /// verify anchors identity on the unforgeable launcher coin, so this must fail closed.
    #[tokio::test]
    async fn impostor_singleton_rejected() -> anyhow::Result<()> {
        use datalayer_driver::{DataStoreInfo, EveProof, Proof};

        let mut sim = Simulator::new();
        let attacker = sim.bls(1);
        let attacker_ph: Bytes32 = attacker.puzzle_hash;

        // The victim store_id the attacker targets. It is NOT a real launched singleton:
        // no launcher coin whose coin_id == store_id exists on the (mock) chain.
        let store_id = Bytes32::new([0xab; 32]);
        let r_parent = Bytes32::new([0x11; 32]);
        let r_evil = Bytes32::new([0xee; 32]);
        // The impostor parent's own parent — an attacker coin, NEVER store_id.
        let attacker_ancestor = Bytes32::new([0xcc; 32]);

        // Forge a singleton currying launcher_id = store_id from an attacker-owned coin.
        let info = DataStoreInfo::new(
            store_id,
            DataStoreMetadata {
                root_hash: r_parent,
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            attacker_ph,
            vec![],
        );
        let impostor_parent_coin = Coin::new(attacker_ancestor, attacker_ph, 1);
        let proof = Proof::Eve(EveProof {
            parent_parent_coin_info: attacker_ancestor,
            parent_amount: 1,
        });
        let impostor_parent = DataStore::new(impostor_parent_coin, proof, info);

        let up = update_store_metadata(
            impostor_parent,
            r_evil,
            None,
            None,
            None,
            None,
            DataStoreInnerSpend::Owner(attacker.pk),
        )
        .map_err(|e| anyhow::anyhow!("forge impostor spend: {e}"))?;
        let forged_parent_spend = up.coin_spends[0].clone();
        let tip_c = up.new_datastore;

        // The pre-#1473 verify TRUSTS exactly this: from_spend yields launcher_id==store_id
        // and root==R_evil — a convincing forgery.
        let mut ctx = SpendContext::new();
        let parsed =
            DataStore::<DataStoreMetadata>::from_spend(&mut ctx, &forged_parent_spend, &[])
                .map_err(|e| anyhow::anyhow!("parse forged spend: {e}"))?
                .expect("forged spend parses as a datastore");
        assert_eq!(
            parsed.info.launcher_id, store_id,
            "the forgery curries the victim launcher_id"
        );
        assert_eq!(
            parsed.info.metadata.root_hash, r_evil,
            "the forgery advertises the attacker's root"
        );

        // Seed the mock as coinset would answer for this attack: tip C unspent + hinted to
        // store_id, its parent P spent with the forged spend, the attacker ancestor present
        // but its own parent NEVER store_id, and — crucially — NO launcher coin for store_id.
        let mut mock = MockChain::default();
        let tip_id = tip_c.coin.coin_id();
        let parent_id = tip_c.coin.parent_coin_info; // == impostor_parent_coin.coin_id()
        mock.records.insert(
            tip_id,
            CoinInfo {
                coin: tip_c.coin,
                spent: false,
                confirmed_block_index: 2,
                spent_block_index: 0,
                timestamp: 0,
                coinbase: false,
            },
        );
        mock.records.insert(
            parent_id,
            CoinInfo {
                coin: impostor_parent_coin,
                spent: true,
                confirmed_block_index: 1,
                spent_block_index: 2,
                timestamp: 0,
                coinbase: false,
            },
        );
        mock.spends.insert(parent_id, forged_parent_spend);
        // The attacker ancestor exists but chains to another attacker coin, never store_id.
        mock.records.insert(
            attacker_ancestor,
            CoinInfo {
                coin: Coin::new(Bytes32::new([0xdd; 32]), attacker_ph, 1),
                spent: true,
                confirmed_block_index: 0,
                spent_block_index: 1,
                timestamp: 0,
                coinbase: false,
            },
        );
        let tip_rec = mock.records.get(&tip_id).expect("tip seeded").clone();
        mock.records_by_hint.insert(store_id, vec![tip_rec]);

        let result = verify_pinned_root(&mock, store_id, r_evil).await;
        assert!(
            result.is_err(),
            "impostor root R_evil must NOT verify — the tip does not descend from a launcher \
             coin whose id == store_id; got {result:?}"
        );
        Ok(())
    }

    /// SECURITY (issue #1473) co-existence variant: a GENUINE store exists for `store_id`
    /// whose forward walk is #747-broken, AND a different-lineage impostor tip currying
    /// `store_id` with `R_evil` is ALSO hinted to `store_id`. The genuine current root must
    /// still verify while the impostor root is rejected.
    #[tokio::test]
    async fn genuine_root_served_impostor_rejected_when_both_hinted() -> anyhow::Result<()> {
        use datalayer_driver::{DataStoreInfo, EveProof, Proof};

        let mut sim = Simulator::new();
        let lin = build_three_gen_store(&mut sim)?;
        let mut mock = seed_mock(&sim, &lin);
        // #747: break the genuine forward walk by dropping an intermediate spend.
        mock.spends.remove(&lin.eve_coin);

        // Forge an impostor tip currying the genuine store_id but from an attacker coin.
        let attacker = sim.bls(1);
        let attacker_ph: Bytes32 = attacker.puzzle_hash;
        let attacker_ancestor = Bytes32::new([0x7c; 32]);
        let r_evil = Bytes32::new([0xee; 32]);
        let info = DataStoreInfo::new(
            lin.store_id,
            DataStoreMetadata {
                root_hash: Bytes32::new([0x71; 32]),
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            attacker_ph,
            vec![],
        );
        let impostor_parent_coin = Coin::new(attacker_ancestor, attacker_ph, 1);
        let impostor_parent = DataStore::new(
            impostor_parent_coin,
            Proof::Eve(EveProof {
                parent_parent_coin_info: attacker_ancestor,
                parent_amount: 1,
            }),
            info,
        );
        let up = update_store_metadata(
            impostor_parent,
            r_evil,
            None,
            None,
            None,
            None,
            DataStoreInnerSpend::Owner(attacker.pk),
        )
        .map_err(|e| anyhow::anyhow!("forge impostor: {e}"))?;
        let forged_parent_spend = up.coin_spends[0].clone();
        let impostor_tip = up.new_datastore;
        let impostor_tip_id = impostor_tip.coin.coin_id();
        let impostor_parent_id = impostor_tip.coin.parent_coin_info;

        mock.records.insert(
            impostor_tip_id,
            CoinInfo {
                coin: impostor_tip.coin,
                spent: false,
                confirmed_block_index: 3,
                spent_block_index: 0,
                timestamp: 0,
                coinbase: false,
            },
        );
        mock.records.insert(
            impostor_parent_id,
            CoinInfo {
                coin: impostor_parent_coin,
                spent: true,
                confirmed_block_index: 2,
                spent_block_index: 3,
                timestamp: 0,
                coinbase: false,
            },
        );
        mock.spends.insert(impostor_parent_id, forged_parent_spend);
        // Both the genuine tip and the impostor tip are hinted to store_id.
        let impostor_rec = mock.records.get(&impostor_tip_id).unwrap().clone();
        mock.records_by_hint
            .get_mut(&lin.store_id)
            .expect("genuine hint seeded")
            .push(impostor_rec);

        // The impostor root is rejected (it never chains back to the launcher coin).
        assert!(
            verify_pinned_root(&mock, lin.store_id, r_evil)
                .await
                .is_err(),
            "impostor root must not be served even alongside the genuine store"
        );
        // The genuine current root still verifies despite the broken forward walk (#747).
        verify_pinned_root(&mock, lin.store_id, lin.root2)
            .await
            .expect("genuine current root verifies");
        Ok(())
    }

    /// FAIL-CLOSED: with no chain data (unreachable / unknown store) verify errors,
    /// never returns Ok.
    #[tokio::test]
    async fn unreachable_chain_fails_closed() {
        let mock = MockChain::default();
        let err = verify_pinned_root(&mock, Bytes32::new([0x01; 32]), Bytes32::new([0x02; 32]))
            .await
            .expect_err("must fail closed when no unspent singleton is found");
        match err {
            ChainError::Chain(msg) => {
                assert!(msg.contains("no current unspent singleton"), "got: {msg}")
            }
            other => panic!("expected fail-closed Chain error, got {other:?}"),
        }
    }
}

// ===========================================================================
// classify_singleton — the #1981 terminal-state classifier suite.
//
// Builds REAL singleton lineages on the in-process Chia simulator (mint /
// update / owner-melt), feeds coinset-shaped reads into the in-crate MockChain,
// and asserts each terminal state is classified EXACTLY. The custody-critical
// property under test is `corrupt_chain_is_err_not_melted`: an unreadable chain
// must NEVER be reported as `Melted`, because downstream a `Melted` authorizes an
// irreversible data delete.
// ===========================================================================
#[cfg(test)]
mod classify_tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::CoinInfo;
    use chia::puzzles::Memos;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{Launcher, SpendContext, StandardLayer};

    /// Seed a MockChain `coin_record` (+ `coin_spend` for spent coins) per id from
    /// the simulator — mirroring the `verify_pinned_root_tests` helper.
    fn seed_records(sim: &Simulator, mock: &mut MockChain, coin_ids: &[Bytes32]) {
        for &coin_id in coin_ids {
            let Some(cs) = sim.coin_state(coin_id) else {
                continue;
            };
            let spent_h = cs.spent_height;
            mock.records.insert(
                coin_id,
                CoinInfo {
                    coin: cs.coin,
                    spent: spent_h.is_some(),
                    confirmed_block_index: cs.created_height.unwrap_or(0),
                    spent_block_index: spent_h.unwrap_or(0),
                    timestamp: 0,
                    coinbase: false,
                },
            );
            if spent_h.is_some() {
                if let Some(spend) = sim.coin_spend(coin_id) {
                    mock.spends.insert(coin_id, spend);
                }
            }
        }
    }

    /// Mint an owner-only store (root0). Returns (store_id, eve DataStore) with every
    /// spend applied on `sim`.
    fn mint_store_on_sim(
        sim: &mut Simulator,
        ctx: &mut SpendContext,
        root0: Bytes32,
    ) -> anyhow::Result<(Bytes32, DataStore, chia_sdk_test::BlsPairWithCoin)> {
        let owner = sim.bls(3);
        let owner_p2 = StandardLayer::new(owner.pk);
        let (launch, eve) = Launcher::new(owner.coin.coin_id(), 1).mint_datastore(
            ctx,
            DataStoreMetadata {
                root_hash: root0,
                label: None,
                description: None,
                bytes: None,
                size_proof: None,
            },
            owner.puzzle_hash.into(),
            vec![],
        )?;
        owner_p2.spend(ctx, owner.coin, launch)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;
        Ok((eve.info.launcher_id, eve, owner))
    }

    /// MELT: launcher spent → eve → eve melt-spent (spend yields no datastore child)
    /// ⇒ `Melted { last_root == root0, melt_spent_height == eve's spent height }`.
    #[tokio::test]
    async fn melted_lineage_classifies_as_melted_with_correct_height() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let root0 = Bytes32::new([0x10; 32]);
        let (store_id, eve, owner) = mint_store_on_sim(&mut sim, ctx, root0)?;

        // Owner-authorized melt of the eve singleton: the spend consumes the
        // singleton and produces NO datastore child.
        let melt_spends =
            melt_store(eve.clone(), owner.pk).map_err(|e| anyhow::anyhow!("melt_store: {e}"))?;
        let sig = sign_coin_spends(&melt_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign melt: {e}"))?;
        sim.new_transaction(SpendBundle::new(melt_spends, sig))?;

        let eve_coin = eve.coin.coin_id();
        let expected_height = sim
            .coin_state(eve_coin)
            .and_then(|cs| cs.spent_height)
            .expect("eve is spent after melt");

        let mut mock = MockChain::default();
        seed_records(&sim, &mut mock, &[store_id, eve_coin]);

        match classify_singleton(&mock, store_id).await? {
            SingletonTerminal::Melted {
                last_root,
                melt_spent_height,
            } => {
                assert_eq!(last_root, root0, "last root before melt is the eve root");
                assert_eq!(
                    melt_spent_height, expected_height,
                    "melt height must be the terminal spend's spent_block_index"
                );
            }
            other => panic!("expected Melted, got {other:?}"),
        }

        // sync_datastore must STILL surface the melt as its legacy error (behaviour
        // unchanged by the refactor).
        let err = sync_datastore(&mock, store_id).await.unwrap_err();
        match err {
            // A melt surfaces through sync_datastore as its legacy generic parse error
            // (melt == Err(DriverError::MissingChild) -> "parse next store: missing child").
            ChainError::Chain(msg) => {
                assert!(msg.contains("missing child"), "got: {msg}")
            }
            other => panic!("expected legacy Chain error, got {other:?}"),
        }
        Ok(())
    }

    /// NEVER-MINTED: launcher coin exists but is UNSPENT ⇒ `NeverMinted`.
    #[tokio::test]
    async fn unspent_launcher_classifies_as_never_minted() -> anyhow::Result<()> {
        let launcher_id = Bytes32::new([0x42; 32]);
        let mut mock = MockChain::default();
        // An unspent launcher coin at the launcher puzzle hash (1 mojo, odd).
        mock.records.insert(
            launcher_id,
            CoinInfo {
                coin: Coin::new(Bytes32::new([0x01; 32]), SINGLETON_LAUNCHER_PH, 1),
                spent: false,
                confirmed_block_index: 100,
                spent_block_index: 0,
                timestamp: 0,
                coinbase: false,
            },
        );

        assert!(matches!(
            classify_singleton(&mock, launcher_id).await?,
            SingletonTerminal::NeverMinted
        ));

        // sync_datastore keeps its legacy "not minted yet" error.
        let err = sync_datastore(&mock, launcher_id).await.unwrap_err();
        match err {
            ChainError::Chain(msg) => assert!(msg.contains("not minted yet"), "got: {msg}"),
            other => panic!("expected legacy Chain error, got {other:?}"),
        }
        Ok(())
    }

    /// LIVE: launcher spent, walk reaches an UNSPENT tip ⇒ `Live(store)` carrying the
    /// current root. Also asserts `sync_datastore` returns a byte-identical store.
    #[tokio::test]
    async fn live_lineage_classifies_as_live() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let root0 = Bytes32::new([0x10; 32]);
        let (store_id, eve, _owner) = mint_store_on_sim(&mut sim, ctx, root0)?;

        // eve is the unspent tip (never updated).
        let eve_coin = eve.coin.coin_id();
        let mut mock = MockChain::default();
        seed_records(&sim, &mut mock, &[store_id, eve_coin]);

        let live = classify_singleton(&mock, store_id).await?;
        let store = match live {
            SingletonTerminal::Live(store) => store,
            other => panic!("expected Live, got {other:?}"),
        };
        assert_eq!(store.info.metadata.root_hash, root0);
        assert_eq!(store.coin.coin_id(), eve_coin);

        // sync_datastore returns the SAME live store (behaviour unchanged).
        let via_sync = sync_datastore(&mock, store_id).await?;
        assert_eq!(via_sync.coin.coin_id(), store.coin.coin_id());
        assert_eq!(
            via_sync.info.metadata.root_hash,
            store.info.metadata.root_hash
        );
        Ok(())
    }

    /// CUSTODY GUARD — corrupt is NEVER melt. The launcher parsed as a datastore
    /// (eve) and eve is SPENT, but eve's spend is UNREADABLE. A missing terminal
    /// spend is the SAME on-chain-read shape a melt would surface through (a spent
    /// terminal coin), so this is the exact case a naive classifier could
    /// mis-report as `Melted`. It MUST stay `Err`.
    #[tokio::test]
    async fn corrupt_terminal_spend_is_err_not_melted() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let root0 = Bytes32::new([0x10; 32]);
        let (store_id, eve, owner) = mint_store_on_sim(&mut sim, ctx, root0)?;

        // Genuinely melt eve so its coin record is SPENT on the sim...
        let melt_spends =
            melt_store(eve.clone(), owner.pk).map_err(|e| anyhow::anyhow!("melt_store: {e}"))?;
        let sig = sign_coin_spends(&melt_spends, std::slice::from_ref(&owner.sk), true)
            .map_err(|e| anyhow::anyhow!("sign melt: {e}"))?;
        sim.new_transaction(SpendBundle::new(melt_spends, sig))?;

        let eve_coin = eve.coin.coin_id();
        let mut mock = MockChain::default();
        seed_records(&sim, &mut mock, &[store_id, eve_coin]);

        // ...then CORRUPT the chain: eve is marked spent but its spend is unreadable.
        // A classifier that inferred melt from "terminal coin is spent" alone (rather
        // than from a spend that parses to Ok(None)) would wrongly return Melted.
        mock.spends.remove(&eve_coin);

        let err = classify_singleton(&mock, store_id)
            .await
            .expect_err("a corrupt/unreadable terminal spend must NOT classify as Melted");
        match err {
            ChainError::Chain(msg) => {
                assert!(msg.contains("singleton spend not found"), "got: {msg}")
            }
            other => panic!("expected Chain error (corrupt), got {other:?}"),
        }
        Ok(())
    }

    /// CUSTODY GUARD — a terminal spend that is PRESENT and parses, but is NOT a
    /// datastore singleton spend (`from_spend` -> `Ok(None)`), is a corrupt terminal,
    /// NOT a melt. Only `Err(DriverError::MissingChild)` is a melt; the `Ok(None)`
    /// arm must fail closed so a garbage terminal spend can never trigger a delete.
    #[tokio::test]
    async fn non_datastore_terminal_spend_is_err_not_melted() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let root0 = Bytes32::new([0x10; 32]);
        let (store_id, eve, _owner) = mint_store_on_sim(&mut sim, ctx, root0)?;
        let eve_coin = eve.coin.coin_id();

        let mut mock = MockChain::default();
        seed_records(&sim, &mut mock, &[store_id, eve_coin]);

        // Mark eve SPENT, but store a NON-datastore spend under its id: a plain
        // standard-layer coin spend. `from_spend` returns Ok(None) (no singleton
        // layer) — which must NOT be read as a melt.
        let victim = sim.bls(0);
        let garbage_spends = {
            let spend_ctx = &mut SpendContext::new();
            StandardLayer::new(victim.pk).spend(
                spend_ctx,
                victim.coin,
                Conditions::new().create_coin(victim.puzzle_hash, 1, Memos::None),
            )?;
            spend_ctx.take()
        };
        let garbage = garbage_spends
            .into_iter()
            .next()
            .expect("one standard coin spend");
        mock.records.insert(
            eve_coin,
            CoinInfo {
                coin: eve.coin,
                spent: true,
                confirmed_block_index: 1,
                spent_block_index: 2,
                timestamp: 0,
                coinbase: false,
            },
        );
        mock.spends.insert(eve_coin, garbage);

        let err = classify_singleton(&mock, store_id)
            .await
            .expect_err("a non-datastore terminal spend must NOT classify as Melted");
        match err {
            ChainError::Chain(msg) => {
                assert!(msg.contains("did not yield a store"), "got: {msg}")
            }
            other => panic!("expected Chain error (corrupt), got {other:?}"),
        }
        Ok(())
    }

    /// CUSTODY GUARD — an unreachable launcher (no coin record at all) is `Err`,
    /// never `Melted` and never `NeverMinted`.
    #[tokio::test]
    async fn unreachable_launcher_is_err_not_melted() {
        let mock = MockChain::default();
        let err = classify_singleton(&mock, Bytes32::new([0x99; 32]))
            .await
            .expect_err("an absent launcher must be an error, not a terminal classification");
        match err {
            ChainError::Chain(msg) => assert!(msg.contains("not found"), "got: {msg}"),
            other => panic!("expected Chain error, got {other:?}"),
        }
    }
}
