//! BIP-39 mnemonic handling and encrypted seed storage.

use crate::error::{ChainError, Result};
use bip39::Mnemonic;
use zeroize::Zeroizing;

/// Validates a BIP-39 mnemonic phrase and returns it normalized.
///
/// Accepts 12/15/18/21/24-word English mnemonics with a valid checksum.
pub fn validate_mnemonic(phrase: &str) -> Result<Zeroizing<String>> {
    let m = Mnemonic::parse(phrase.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    Ok(Zeroizing::new(m.to_string()))
}

/// Generates a new BIP-39 mnemonic with the given word count (12/15/18/21/24).
pub fn generate_mnemonic(word_count: usize) -> Result<Zeroizing<String>> {
    let m = Mnemonic::generate(word_count)
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    Ok(Zeroizing::new(m.to_string()))
}

#[cfg(test)]
mod mnemonic_tests {
    use super::*;

    const VALID_24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn valid_24_word_parses() {
        let m = validate_mnemonic(VALID_24).unwrap();
        assert_eq!(m.split_whitespace().count(), 24);
    }

    #[test]
    fn invalid_word_rejected() {
        let bad = VALID_24.replace("art", "zzzzzz");
        assert!(matches!(validate_mnemonic(&bad), Err(ChainError::InvalidMnemonic(_))));
    }

    #[test]
    fn bad_checksum_rejected() {
        // 24 valid words but wrong checksum (last word swapped to another valid word).
        let bad = VALID_24.replace("art", "abandon");
        assert!(matches!(validate_mnemonic(&bad), Err(ChainError::InvalidMnemonic(_))));
    }

    #[test]
    fn generate_24_round_trips() {
        let m = generate_mnemonic(24).unwrap();
        assert_eq!(m.split_whitespace().count(), 24);
        // Generated mnemonic must itself validate.
        assert!(validate_mnemonic(&m).is_ok());
    }

    #[test]
    fn generate_rejects_bad_word_count() {
        assert!(generate_mnemonic(13).is_err());
    }
}