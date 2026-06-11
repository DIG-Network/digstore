//! Build + sign Chia datastore singleton spends (mint here; update in a later
//! task). Pure: callers fetch unspent coins via `ChainReads` and broadcast the
//! returned bundle via `ChainReads::push`. Verified on mainnet in the Phase-0
//! prototype.

use crate::error::{ChainError, Result};
use crate::keys::WalletKeys;
use datalayer_driver::{
    mint_store, select_coins, sign_coin_spends, Bytes32, Coin, DataStore, SpendBundle,
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
    let signature = sign_coin_spends(&coin_spends, &[keys.synthetic_sk.clone()], false)
        .map_err(|e| ChainError::Chain(format!("sign: {e}")))?;
    let bundle = SpendBundle::new(coin_spends, signature);
    Ok(MintBuild { bundle, launcher_id, datastore: new_datastore })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_wallet_keys;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

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
