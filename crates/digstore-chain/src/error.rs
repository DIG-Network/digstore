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
    /// The wallet holds enough total value of `asset` but too many coins: reaching
    /// the target would need more than `cap` inputs, so the spend bundle would exceed
    /// Chia's block-cost ceiling. The spend cannot be built until the wallet is
    /// CONSOLIDATED (coins merged into fewer, larger ones). DISTINCT from a genuine
    /// funding shortfall (consolidation cannot create value) and from
    /// [`BundleTooLarge`](Self::BundleTooLarge) (which is detected only after a build):
    /// this is decided up front by the capped selector. The CLI catches it to run the
    /// consolidate → confirm → retry loop (coin-management epic #410).
    #[error(
        "{asset} is spendable but too fragmented: {available_coin_count} coins, \
         the largest {cap} cannot cover {required} mojos — consolidate first"
    )]
    NeedsConsolidation {
        /// The asset needing consolidation (`"XCH"` or a 0x-prefixed CAT tail hex).
        asset: String,
        /// Total number of unspent coins of the asset the wallet holds.
        available_coin_count: u32,
        /// Sum of all unspent coins of the asset in mojos (always `>= required`).
        available_total: u64,
        /// The target that could not be reached within `cap`, in mojos.
        required: u64,
        /// The coin-count cap in force for the selection.
        cap: usize,
    },
}

pub type Result<T> = std::result::Result<T, ChainError>;
