use crate::error::Result;
use digstore_core::ChiaBlockRef;

/// A source of Chia chain state for anchoring proofs to wall-clock time
/// (§13.8). Implemented by [`crate::mock_chain::MockChainSource`] and
/// [`crate::coinset::CoinsetChainSource`].
pub trait ChainSource {
    /// The current peak block of the trusted chain.
    fn get_peak(&self) -> Result<ChiaBlockRef>;

    /// Confirm `block` is a real block on the trusted chain AND that its
    /// timestamp falls within `freshness_window_secs` of "now". Rejects blocks
    /// that are unknown, too old, or in the future.
    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()>;
}
