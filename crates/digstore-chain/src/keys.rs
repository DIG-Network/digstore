//! Derive Chia wallet keys (synthetic key + owner puzzle hash) from a BIP-39
//! mnemonic. Matches the standard Chia wallet first-receive key, so the owner
//! puzzle hash is where the wallet's XCH actually resides. Verified against a
//! live mainnet wallet in the Phase-0 prototype.

use crate::error::{ChainError, Result};
use bip39::Mnemonic;
use chia::puzzles::{standard::StandardArgs, DeriveSynthetic};
use datalayer_driver::{
    master_public_key_to_first_puzzle_hash, master_secret_key_to_wallet_synthetic_secret_key,
    master_to_wallet_unhardened, puzzle_hash_to_address, secret_key_to_public_key, Bytes32,
    PublicKey, SecretKey,
};
use zeroize::Zeroizing;

/// Wallet keys derived from the seed. `synthetic_sk` signs spends; coins are
/// held at `owner_puzzle_hash` (= standard puzzle of `synthetic_pk`).
pub struct WalletKeys {
    pub synthetic_sk: SecretKey,
    pub synthetic_pk: PublicKey,
    pub owner_puzzle_hash: Bytes32,
}

/// Derives wallet keys from a validated BIP-39 mnemonic.
pub fn derive_wallet_keys(mnemonic: &str) -> Result<WalletKeys> {
    let m = Mnemonic::parse_normalized(mnemonic.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    let seed = Zeroizing::new(m.to_seed("")); // [u8; 64]; zeroized on drop
    let master_sk = SecretKey::from_seed(seed.as_ref());
    let master_pk = master_sk.public_key();
    let owner_puzzle_hash = master_public_key_to_first_puzzle_hash(&master_pk);
    let synthetic_sk = master_secret_key_to_wallet_synthetic_secret_key(&master_sk);
    let synthetic_pk = secret_key_to_public_key(&synthetic_sk);
    Ok(WalletKeys {
        synthetic_sk,
        synthetic_pk,
        owner_puzzle_hash,
    })
}

/// Derive wallet keys from a RAW 32-byte master seed (not a BIP-39 mnemonic).
///
/// Used by the #17 writer-delegate (deploy-token) path: a deploy token is a raw
/// 32-byte seed (the same form `deploy-key export` emits), and the on-chain writer
/// authorization needs that seed's WALLET SYNTHETIC key — the standard wallet
/// derivation (`master → wallet synthetic`), so the curried writer puzzle hash
/// (`StandardArgs::curry_tree_hash(synthetic_pk)`) matches what the owner
/// delegated. The returned `owner_puzzle_hash` is the writer's own p2 standard
/// puzzle hash (the writer delegate is not the store owner; it never holds coins).
pub fn wallet_keys_from_seed(seed: &[u8; 32]) -> WalletKeys {
    let master_sk = SecretKey::from_seed(seed);
    let synthetic_sk = master_secret_key_to_wallet_synthetic_secret_key(&master_sk);
    let synthetic_pk = secret_key_to_public_key(&synthetic_sk);
    let owner_puzzle_hash = StandardArgs::curry_tree_hash(synthetic_pk).into();
    WalletKeys {
        synthetic_sk,
        synthetic_pk,
        owner_puzzle_hash,
    }
}

/// Wallet keys for a single unhardened HD index. Index 0 byte-matches
/// `derive_wallet_keys` (the legacy single-address path).
#[derive(Clone)]
pub struct IndexedKeys {
    pub index: u32,
    pub synthetic_sk: SecretKey,
    pub synthetic_pk: PublicKey,
    pub owner_puzzle_hash: Bytes32,
}

/// Derive the wallet keys for a range of unhardened indices.
///
/// For each index `i`, the path is the standard Chia unhardened wallet path
/// (`m/12381/8444/2/i`), with the synthetic key offset applied
/// (`DEFAULT_HIDDEN_PUZZLE_HASH`), yielding the p2-standard puzzle hash.
///
/// Index 0 byte-matches `derive_wallet_keys` — both use
/// `master_to_wallet_unhardened(key, 0).derive_synthetic()`.
pub fn derive_indexed_keys(
    mnemonic: &str,
    indices: std::ops::Range<u32>,
) -> Result<Vec<IndexedKeys>> {
    let m = Mnemonic::parse_normalized(mnemonic.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    let seed = Zeroizing::new(m.to_seed(""));
    let master_sk = SecretKey::from_seed(seed.as_ref());
    let mut out = Vec::new();
    for index in indices {
        let synthetic_sk = master_to_wallet_unhardened(&master_sk, index).derive_synthetic();
        let synthetic_pk = secret_key_to_public_key(&synthetic_sk);
        let owner_puzzle_hash = StandardArgs::curry_tree_hash(synthetic_pk).into();
        out.push(IndexedKeys {
            index,
            synthetic_sk,
            synthetic_pk,
            owner_puzzle_hash,
        });
    }
    Ok(out)
}

/// The owner's mainnet receive address (`xch1…`), bech32m-encoded from the
/// owner puzzle hash. This is where the wallet's XCH resides, so it is the
/// address surfaced in an `InsufficientFunds` error for funding.
///
/// A valid 32-byte puzzle hash always encodes successfully; if encoding ever
/// failed (it should not), the hex puzzle hash is returned as a last-resort
/// fallback so the user still has something actionable.
pub fn owner_address(keys: &WalletKeys) -> String {
    puzzle_hash_to_address(keys.owner_puzzle_hash, "xch").unwrap_or_else(|_| {
        keys.owner_puzzle_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn derives_stable_owner_puzzle_hash() {
        let k = derive_wallet_keys(ABANDON).unwrap();
        // GOLDEN: captured from the ABANDON BIP-39 test vector (Phase-0 derivation path).
        assert_eq!(
            hex::encode(k.owner_puzzle_hash),
            "d207c1e11fc3b0cd7472e8c7e53c8d2b81709516346c7baa9fbb9070ffccfe89"
        );
    }

    #[test]
    fn rejects_bad_mnemonic() {
        assert!(matches!(
            derive_wallet_keys("not a real mnemonic"),
            Err(ChainError::InvalidMnemonic(_))
        ));
    }

    #[test]
    fn owner_address_is_nonempty_xch() {
        let k = derive_wallet_keys(ABANDON).unwrap();
        let addr = owner_address(&k);
        assert!(addr.starts_with("xch1"), "got address: {addr}");
        assert!(addr.len() > 4);
    }

    #[test]
    fn deterministic() {
        let a = derive_wallet_keys(ABANDON).unwrap();
        let b = derive_wallet_keys(ABANDON).unwrap();
        assert_eq!(a.owner_puzzle_hash, b.owner_puzzle_hash);
        assert_eq!(a.synthetic_pk.to_bytes(), b.synthetic_pk.to_bytes());
    }

    #[test]
    fn wallet_keys_from_seed_is_deterministic_and_distinct() {
        // #17: a raw 32-byte deploy-token seed derives a stable wallet synthetic key.
        let seed = [7u8; 32];
        let a = wallet_keys_from_seed(&seed);
        let b = wallet_keys_from_seed(&seed);
        assert_eq!(a.synthetic_pk.to_bytes(), b.synthetic_pk.to_bytes());
        assert_eq!(a.owner_puzzle_hash, b.owner_puzzle_hash);
        // A different seed yields a different key (so a writer delegate is its own identity).
        let c = wallet_keys_from_seed(&[8u8; 32]);
        assert_ne!(a.synthetic_pk.to_bytes(), c.synthetic_pk.to_bytes());
    }

    #[test]
    fn indexed_keys_index0_matches_single() {
        let single = derive_wallet_keys(ABANDON).unwrap();
        let many = derive_indexed_keys(ABANDON, 0..3).unwrap();
        assert_eq!(many.len(), 3);
        assert_eq!(many[0].index, 0);
        assert_eq!(many[0].owner_puzzle_hash, single.owner_puzzle_hash);
        // distinct addresses per index
        assert_ne!(many[0].owner_puzzle_hash, many[1].owner_puzzle_hash);
        assert_ne!(many[1].owner_puzzle_hash, many[2].owner_puzzle_hash);
    }
}
