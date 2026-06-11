//! Error type for seed/chain operations.

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("no seed found at {0}")]
    NoSeed(String),
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
    #[error("decryption failed (wrong passphrase or corrupt seed file)")]
    Decrypt,
    #[error("malformed seed file: {0}")]
    MalformedSeedFile(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("chain error: {0}")]
    Chain(String),
}

pub type Result<T> = std::result::Result<T, ChainError>;
