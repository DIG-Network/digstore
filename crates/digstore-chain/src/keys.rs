//! Derive Chia wallet keys (synthetic key + owner puzzle hash) from a BIP-39
//! mnemonic. Matches the standard Chia wallet first-receive key, so the owner
//! puzzle hash is where the wallet's XCH actually resides. Verified against a
//! live mainnet wallet in the Phase-0 prototype.

use crate::error::{ChainError, Result};
use bip39::Mnemonic;
use datalayer_driver::{
    master_public_key_to_first_puzzle_hash, master_secret_key_to_wallet_synthetic_secret_key,
    puzzle_hash_to_address, secret_key_to_public_key, Bytes32, PublicKey, SecretKey,
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
    Ok(WalletKeys { synthetic_sk, synthetic_pk, owner_puzzle_hash })
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
}
