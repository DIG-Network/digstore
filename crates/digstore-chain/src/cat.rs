//! DIG CAT coins over coinset: locate + value the wallet's DIG (spend comes later).
use crate::coinset::ChainReads;
use crate::dig::DIG_ASSET_ID;
use crate::error::Result;
use chia::puzzles::cat::CatArgs;
use chia_protocol::Bytes32;
use chia_wallet_sdk::prelude::TreeHash;

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
}
