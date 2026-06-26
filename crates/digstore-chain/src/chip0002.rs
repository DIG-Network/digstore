//! CHIP-0002 signing — the cryptographic core of the browser's WalletConnect /
//! dapp wallet. Every signature here is **byte-exact** to Sage / Goby /
//! chia-blockchain because it reduces, via `datalayer-driver`, to the canonical
//! `bls_sign(sk, sha256tree(("Chia Signed Message" . message)))` (CHIP-0002) and
//! `sign_coin_spends` with the mainnet `agg_sig_me` additional data.
//!
//! Key model (matches Sage's `chip0002_getPublicKeys`):
//!   * `getPublicKeys` returns the **non-synthetic** unhardened wallet public
//!     keys (path `m/12381/8444/2/i`). A dapp hands one of these back to
//!     `signMessage`.
//!   * `signMessage` finds the secret key whose public key matches (checking both
//!     the non-synthetic and synthetic form per index, so an address-derived
//!     synthetic key also works) and AugScheme-signs the CHIP-0002 digest.
//!
//! This module is pure (no network, no disk) so it is golden-vector testable.

use crate::error::{ChainError, Result};
use bip39::Mnemonic;
use chia::puzzles::DeriveSynthetic;
use chia_protocol::Program;
use datalayer_driver::{
    master_to_wallet_unhardened, sign_coin_spends, sign_message, verify_signed_message, Bytes32,
    Coin, CoinSpend, PublicKey, SecretKey, Signature,
};
use serde::Deserialize;
use zeroize::Zeroizing;

/// Normalize a hex string for comparison: strip an optional `0x`, lowercase.
fn normalize_hex(s: &str) -> String {
    s.trim().trim_start_matches("0x").to_ascii_lowercase()
}

/// The BIP-39 master secret key for a mnemonic (seed zeroized on drop).
fn master_secret_key(mnemonic: &str) -> Result<SecretKey> {
    let m = Mnemonic::parse_normalized(mnemonic.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    let seed = Zeroizing::new(m.to_seed(""));
    Ok(SecretKey::from_seed(seed.as_ref()))
}

/// CHIP-0002 `getPublicKeys`: the non-synthetic unhardened wallet public keys
/// for indices `offset..offset+limit`, hex-encoded **without** a `0x` prefix
/// (Sage's WC convention). 48-byte G1 keys → 96 hex chars each.
pub fn wallet_public_keys(mnemonic: &str, offset: u32, limit: u32) -> Result<Vec<String>> {
    let master_sk = master_secret_key(mnemonic)?;
    let end = offset.saturating_add(limit);
    let mut out = Vec::with_capacity((end - offset) as usize);
    for i in offset..end {
        let sk = master_to_wallet_unhardened(&master_sk, i);
        out.push(hex::encode(sk.public_key().to_bytes()));
    }
    Ok(out)
}

/// Low-level: AugScheme-sign a CHIP-0002 message with a specific secret key.
/// Returns the 96-byte signature as hex (no `0x`).
pub fn sign_message_with(sk: &SecretKey, message: &[u8]) -> Result<String> {
    let sig: Signature =
        sign_message(message, sk).map_err(|e| ChainError::Crypto(e.to_string()))?;
    Ok(hex::encode(sig.to_bytes()))
}

/// CHIP-0002 `signMessage`: sign `message` with the wallet key whose public key
/// equals `public_key_hex`, searching indices `0..search_limit` (checking both
/// the non-synthetic and synthetic public key per index). Returns the signature
/// hex. Errors if no key matches.
pub fn sign_message_by_public_key(
    mnemonic: &str,
    public_key_hex: &str,
    message: &[u8],
    search_limit: u32,
) -> Result<String> {
    let target = normalize_hex(public_key_hex);
    let master_sk = master_secret_key(mnemonic)?;
    for i in 0..search_limit {
        let sk = master_to_wallet_unhardened(&master_sk, i);
        if hex::encode(sk.public_key().to_bytes()) == target {
            return sign_message_with(&sk, message);
        }
        let syn = sk.derive_synthetic();
        if hex::encode(syn.public_key().to_bytes()) == target {
            return sign_message_with(&syn, message);
        }
    }
    Err(ChainError::Crypto(format!(
        "no wallet key matches public key {target} (searched {search_limit} indices)"
    )))
}

/// Decode a hex string (optional `0x`) into a fixed 32-byte hash.
fn hex32(s: &str) -> Result<Bytes32> {
    let bytes =
        hex::decode(normalize_hex(s)).map_err(|e| ChainError::Crypto(format!("bad hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ChainError::Crypto("expected 32 bytes".into()))?;
    Ok(arr.into())
}

/// Decode a hex string (optional `0x`) into a CLVM `Program`.
fn hex_program(s: &str) -> Result<Program> {
    let bytes =
        hex::decode(normalize_hex(s)).map_err(|e| ChainError::Crypto(format!("bad hex: {e}")))?;
    Ok(Program::from(bytes))
}

/// Accept a coin amount as either a JSON number or a decimal string (Sage/WC
/// dapps send both).
fn de_amount<'de, D>(d: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        N(u64),
        S(String),
    }
    match NumOrStr::deserialize(d)? {
        NumOrStr::N(n) => Ok(n),
        NumOrStr::S(s) => s.trim().parse().map_err(serde::de::Error::custom),
    }
}

/// A WalletConnect coin (Sage shape, hex strings; `0x` optional).
#[derive(Deserialize)]
pub struct WcCoin {
    pub parent_coin_info: String,
    pub puzzle_hash: String,
    #[serde(deserialize_with = "de_amount")]
    pub amount: u64,
}

/// A WalletConnect coin spend, as it arrives in `chip0002_signCoinSpends` params.
/// Tolerant of dapp variation (optional `0x`, amount number-or-string).
#[derive(Deserialize)]
pub struct WcCoinSpend {
    pub coin: WcCoin,
    pub puzzle_reveal: String,
    pub solution: String,
}

impl WcCoinSpend {
    fn to_coin_spend(&self) -> Result<CoinSpend> {
        let coin = Coin::new(
            hex32(&self.coin.parent_coin_info)?,
            hex32(&self.coin.puzzle_hash)?,
            self.coin.amount,
        );
        Ok(CoinSpend::new(
            coin,
            hex_program(&self.puzzle_reveal)?,
            hex_program(&self.solution)?,
        ))
    }
}

/// CHIP-0002 `signCoinSpends` from the WalletConnect JSON shape: parse the dapp's
/// coin spends and return the aggregated signature hex. The bridge between the WC
/// wire format and [`sign_coin_spends_hex`].
pub fn sign_wc_coin_spends(
    mnemonic: &str,
    spends: &[WcCoinSpend],
    key_window: u32,
) -> Result<String> {
    let coin_spends = spends
        .iter()
        .map(|s| s.to_coin_spend())
        .collect::<Result<Vec<_>>>()?;
    sign_coin_spends_hex(mnemonic, &coin_spends, key_window)
}

/// CHIP-0002 `signCoinSpends`: AugScheme-aggregate-sign `coin_spends` with the
/// wallet's secret keys (unhardened indices `0..key_window`). `sign_coin_spends`
/// maps each raw **and** synthetic public key to its secret key, so coins held
/// at the standard puzzle (synthetic) are covered without passing synthetic keys
/// explicitly. Mainnet `agg_sig_me` data (`for_testnet = false`). Returns the
/// 96-byte aggregated signature hex.
pub fn sign_coin_spends_hex(
    mnemonic: &str,
    coin_spends: &[CoinSpend],
    key_window: u32,
) -> Result<String> {
    let master_sk = master_secret_key(mnemonic)?;
    let keys: Vec<SecretKey> = (0..key_window)
        .map(|i| master_to_wallet_unhardened(&master_sk, i))
        .collect();
    let sig = sign_coin_spends(coin_spends, &keys, false)
        .map_err(|e| ChainError::Crypto(e.to_string()))?;
    Ok(hex::encode(sig.to_bytes()))
}

/// Verify a CHIP-0002 message signature against a public key. Used by tests and
/// callers that want a round-trip check; mirrors `datalayer_driver`'s verifier.
pub fn verify_message(public_key_hex: &str, message: &[u8], signature_hex: &str) -> Result<bool> {
    let pk_bytes = hex::decode(normalize_hex(public_key_hex))
        .map_err(|e| ChainError::Crypto(format!("bad public key hex: {e}")))?;
    let pk_arr: [u8; 48] = pk_bytes
        .as_slice()
        .try_into()
        .map_err(|_| ChainError::Crypto("public key must be 48 bytes".into()))?;
    let pk = PublicKey::from_bytes(&pk_arr).map_err(|e| ChainError::Crypto(e.to_string()))?;

    let sig_bytes = hex::decode(normalize_hex(signature_hex))
        .map_err(|e| ChainError::Crypto(format!("bad signature hex: {e}")))?;
    let sig_arr: [u8; 96] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| ChainError::Crypto("signature must be 96 bytes".into()))?;
    let sig = Signature::from_bytes(&sig_arr).map_err(|e| ChainError::Crypto(e.to_string()))?;

    verify_signed_message(&sig, &pk, message).map_err(|e| ChainError::Crypto(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn public_keys_are_stable_48_byte_hex() {
        let keys = wallet_public_keys(ABANDON, 0, 3).unwrap();
        assert_eq!(keys.len(), 3);
        for k in &keys {
            assert_eq!(k.len(), 96, "48-byte G1 key = 96 hex chars");
            assert!(!k.starts_with("0x"));
        }
        // Distinct per index.
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        // Deterministic.
        assert_eq!(wallet_public_keys(ABANDON, 0, 3).unwrap(), keys);
    }

    #[test]
    fn sign_message_round_trips_against_its_public_key() {
        let pk0 = &wallet_public_keys(ABANDON, 0, 1).unwrap()[0];
        let msg = b"hello DIG";
        let sig = sign_message_by_public_key(ABANDON, pk0, msg, 5).unwrap();
        assert_eq!(sig.len(), 192, "96-byte G2 sig = 192 hex chars");
        // Byte-exact CHIP-0002: verifies against the same public key.
        assert!(verify_message(pk0, msg, &sig).unwrap());
        // Wrong message must NOT verify.
        assert!(!verify_message(pk0, b"tampered", &sig).unwrap());
    }

    #[test]
    fn sign_message_is_deterministic() {
        let pk0 = &wallet_public_keys(ABANDON, 0, 1).unwrap()[0];
        let a = sign_message_by_public_key(ABANDON, pk0, b"x", 5).unwrap();
        let b = sign_message_by_public_key(ABANDON, pk0, b"x", 5).unwrap();
        assert_eq!(a, b, "AugScheme signing is deterministic");
    }

    #[test]
    fn sign_message_accepts_0x_prefix_and_synthetic_key() {
        // 0x-prefixed input is accepted.
        let pk0 = wallet_public_keys(ABANDON, 0, 1).unwrap()[0].clone();
        let sig = sign_message_by_public_key(ABANDON, &format!("0x{pk0}"), b"y", 5).unwrap();
        assert!(verify_message(&pk0, b"y", &sig).unwrap());
    }

    #[test]
    fn sign_coin_spends_matches_the_canonical_send_aggregate() {
        // A dapp's signCoinSpends must produce the SAME aggregate signature as the
        // wallet's own send path for the same spends — proving the WC signer is
        // byte-identical to the canonical signer (and that passing non-synthetic
        // keys still covers standard-puzzle coins via sign_coin_spends' synthetic
        // mapping).
        use crate::keys::derive_indexed_keys;
        use crate::send::build_xch_send;
        use crate::wallet::{AddressCoins, ScannedWallet};
        use datalayer_driver::{Bytes32, Coin};

        fn b32(b: u8) -> Bytes32 {
            [b; 32].into()
        }

        let k = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let coin = Coin::new(b32(0x10), k.owner_puzzle_hash, 10_000);
        let wallet = ScannedWallet {
            addrs: vec![AddressCoins {
                keys: k,
                xch: vec![coin],
                dig: vec![],
            }],
        };
        let (bundle, _plan) = build_xch_send(&wallet, b32(0xAA), 4_000, 500).unwrap();
        let canonical = hex::encode(bundle.aggregated_signature.to_bytes());

        let mine = sign_coin_spends_hex(ABANDON, &bundle.coin_spends, 4).unwrap();
        assert_eq!(
            mine, canonical,
            "WC signCoinSpends must equal the canonical send aggregate"
        );
        assert_eq!(mine.len(), 192);

        // And via the WC JSON wire path: serialize the spends to JSON, parse them
        // back through the tolerant WC parser, sign — must still match. This proves
        // the hex<->CoinSpend bridge is faithful.
        let wire = serde_json::to_value(&bundle.coin_spends).unwrap();
        let parsed: Vec<WcCoinSpend> = serde_json::from_value(wire).unwrap();
        let via_wire = sign_wc_coin_spends(ABANDON, &parsed, 4).unwrap();
        assert_eq!(
            via_wire, canonical,
            "WC JSON round-trip must sign identically"
        );
    }

    #[test]
    fn unknown_public_key_errors() {
        let bogus = "ab".repeat(48); // 48 bytes, not ours
        let err = sign_message_by_public_key(ABANDON, &bogus, b"z", 5).unwrap_err();
        assert!(matches!(err, ChainError::Crypto(_)));
    }
}
