//! Build + sign Chia datastore singleton spends (mint + update). Pure: callers
//! fetch unspent coins via `ChainReads` and broadcast the returned bundle via
//! `ChainReads::push`. Verified on mainnet in the Phase-0 prototype.

use crate::coinset::ChainReads;
use crate::error::{ChainError, Result};
use crate::keys::WalletKeys;
use chia_wallet_sdk::driver::SpendContext;
use datalayer_driver::{
    add_fee, mint_store, select_coins, sign_coin_spends, update_store_metadata, Bytes32, Coin,
    DataStore, DataStoreInnerSpend, DataStoreMetadata, DelegatedPuzzle, SpendBundle,
    SuccessResponse,
};

/// A built, signed mint ready to broadcast.
pub struct MintBuild {
    pub bundle: SpendBundle,
    pub launcher_id: Bytes32,
    pub datastore: DataStore,
}

/// Builds + signs a mint of an owner-only empty/initial store with `root`.
/// `unspent` are the wallet's spendable XCH coins; `fee` in mojos.
pub fn build_mint(
    keys: &WalletKeys,
    unspent: &[Coin],
    root: Bytes32,
    fee: u64,
) -> Result<MintBuild> {
    let selected = select_coins(unspent, fee + 1)
        .map_err(|e| ChainError::Chain(format!("select_coins: {e}")))?;
    let SuccessResponse { coin_spends, new_datastore } = mint_store(
        keys.synthetic_pk,
        selected,
        root,
        None,
        None,
        None,
        None,
        keys.owner_puzzle_hash,
        vec![],
        fee,
    )
    .map_err(|e| ChainError::Chain(format!("mint_store: {e}")))?;
    let launcher_id = new_datastore.info.launcher_id;
    let signature = sign_coin_spends(&coin_spends, std::slice::from_ref(&keys.synthetic_sk), false)
        .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    let bundle = SpendBundle::new(coin_spends, signature);
    Ok(MintBuild { bundle, launcher_id, datastore: new_datastore })
}

/// A built, signed store-root update ready to broadcast.
pub struct UpdateBuild {
    pub bundle: SpendBundle,
    pub new_coin_id: Bytes32,
    pub datastore: DataStore,
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
    let SuccessResponse { coin_spends: update_spends, new_datastore } = update_store_metadata(
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

    let signature =
        sign_coin_spends(&coin_spends, std::slice::from_ref(&keys.synthetic_sk), false)
            .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    let new_coin_id = new_datastore.coin.coin_id();
    let bundle = SpendBundle::new(coin_spends, signature);
    Ok(UpdateBuild { bundle, new_coin_id, datastore: new_datastore })
}

/// Reconstructs the current unspent datastore singleton for `launcher_id` using
/// only coinset reads (coin records + puzzle/solution), following the singleton
/// lineage. No P2P peer required. Owner-only stores carry no delegated puzzles.
///
/// `DataStore::from_spend(ctx, spend, delegated)` returns the CHILD datastore
/// created by spending `spend.coin`, so we walk launcher -> eve -> ... forward
/// until we reach a singleton coin that is still unspent.
pub async fn sync_datastore(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
) -> Result<DataStore> {
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

#[cfg(test)]
mod sync_tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::Coinset;

    fn launcher_bytes32() -> Bytes32 {
        let raw =
            hex::decode("cf915cbaac0755db8c79b1b2e3b2eadf14d14f7246bb7e05d951802cd273211c")
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
