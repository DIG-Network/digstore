use crate::chain::ChainSource;
use crate::error::{ProverError, Result};
use digstore_core::ChiaBlockRef;
use std::collections::HashMap;

/// Deterministic in-memory [`ChainSource`] for tests. Holds a fixed set of
/// known blocks (keyed by header hash) and a fixed `now`.
#[derive(Debug, Clone)]
pub struct MockChainSource {
    blocks: HashMap<[u8; 32], ChiaBlockRef>,
    peak: ChiaBlockRef,
    now: u64,
}

impl MockChainSource {
    /// `blocks[0]` is treated as the peak. `now` is the fixed wall clock.
    pub fn new(blocks: Vec<ChiaBlockRef>, now: u64) -> Self {
        assert!(!blocks.is_empty(), "MockChainSource needs at least one block");
        let peak = blocks[0].clone();
        let map = blocks.into_iter().map(|b| (b.header_hash.0, b)).collect();
        Self { blocks: map, peak, now }
    }

    /// Override the fixed "now" used for freshness checks.
    pub fn with_now(mut self, now: u64) -> Self {
        self.now = now;
        self
    }
}

impl ChainSource for MockChainSource {
    fn get_peak(&self) -> Result<ChiaBlockRef> {
        Ok(self.peak.clone())
    }

    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()> {
        match self.blocks.get(&block.header_hash.0) {
            Some(b) if b == block => {}
            _ => return Err(ProverError::BlockNotOnChain(block.header_hash.to_hex())),
        }
        if block.timestamp > self.now {
            return Err(ProverError::BlockInFuture(block.timestamp, self.now));
        }
        if self.now - block.timestamp > freshness_window_secs {
            return Err(ProverError::BlockTooOld {
                block_ts: block.timestamp,
                now: self.now,
                window: freshness_window_secs,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    fn block(h: u32, ts: u64, tag: u8) -> ChiaBlockRef {
        ChiaBlockRef { header_hash: Bytes32([tag; 32]), height: h, timestamp: ts }
    }

    #[test]
    fn get_peak_returns_configured_peak() {
        let peak = block(100, 1_000, 0x01);
        let src = MockChainSource::new(vec![peak.clone()], 1_000);
        assert_eq!(src.get_peak().unwrap(), peak);
    }

    #[test]
    fn verify_block_accepts_known_fresh_block() {
        let b = block(100, 990, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(src.verify_block(&b, 60).is_ok());
    }

    #[test]
    fn verify_block_rejects_stale_block() {
        let b = block(100, 900, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(matches!(src.verify_block(&b, 60).unwrap_err(), ProverError::BlockTooOld { .. }));
    }

    #[test]
    fn verify_block_rejects_unknown_block() {
        let src = MockChainSource::new(vec![block(100, 990, 0x01)], 1_000);
        let unknown = block(101, 995, 0x02);
        assert!(matches!(src.verify_block(&unknown, 60).unwrap_err(), ProverError::BlockNotOnChain(_)));
    }

    #[test]
    fn verify_block_rejects_future_block() {
        let b = block(100, 1_050, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(matches!(src.verify_block(&b, 60).unwrap_err(), ProverError::BlockInFuture(_, _)));
    }

    #[test]
    fn with_now_overrides_clock() {
        let b = block(100, 990, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000).with_now(2_000);
        assert!(matches!(src.verify_block(&b, 60).unwrap_err(), ProverError::BlockTooOld { .. }));
    }
}
