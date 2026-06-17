//! Adaptive HD wallet scan: derive addresses in chunks and aggregate the wallet's
//! XCH + DIG coins across all of them (Sage-style whole-wallet balance), so the CLI
//! no longer sees only index 0.

use crate::cat::dig_cat_puzzle_hash;
use crate::coinset::ChainReads;
use crate::error::Result;
use crate::keys::{derive_indexed_keys, IndexedKeys};
use chia_protocol::Coin;
use datalayer_driver::SecretKey;

const CHUNK: u32 = 50;
const MAX_INDEX: u32 = 500;

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
}
