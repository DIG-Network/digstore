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
    /// A spend bundle is too large to broadcast (its CLVM cost / generator size would exceed Chia's
    /// per-block limit). This is a TERMINAL, actionable condition — retrying cannot help and it is
    /// NOT a coinset.org connectivity problem (#231). The operation must be split into smaller
    /// batches (e.g. `collection mint --batch-size`).
    #[error("spend bundle too large: {0}")]
    BundleTooLarge(String),
}

pub type Result<T> = std::result::Result<T, ChainError>;
