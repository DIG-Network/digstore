//! Injectable wall-clock source for `host_get_current_time` (§12).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of the current Unix time in seconds. Injectable so tests are
/// deterministic and temporal-key checks (§16) are reproducible.
pub trait Clock: Send + Sync + 'static {
    fn now_unix_secs(&self) -> u64;
}

/// Production clock backed by the OS wall clock.
///
/// This is the REAL clock for residual #3 (`SECURITY.md`): the blind serve path
/// injects it via [`crate::serve_blind::BlindServeDeps::with_real_chain_clock`]
/// (replacing the deterministic `FixedClock`) whenever a real chain source is
/// wired, so temporal-key / freshness checks (§16) run against actual time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Deterministic clock for tests; time only moves when `advance`/`set` is called.
/// Clone shares the same underlying counter.
#[derive(Debug, Clone)]
pub struct FixedClock(Arc<AtomicU64>);

impl FixedClock {
    pub fn new(secs: u64) -> Self {
        FixedClock(Arc::new(AtomicU64::new(secs)))
    }
    pub fn advance(&self, secs: u64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
    pub fn set(&self, secs: u64) {
        self.0.store(secs, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_unix_secs(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_injected_time() {
        let clock = FixedClock::new(1_700_000_000);
        assert_eq!(clock.now_unix_secs(), 1_700_000_000);
    }

    #[test]
    fn fixed_clock_can_advance() {
        let clock = FixedClock::new(100);
        clock.advance(50);
        assert_eq!(clock.now_unix_secs(), 150);
    }

    #[test]
    fn fixed_clock_set_overwrites() {
        let clock = FixedClock::new(100);
        clock.set(999);
        assert_eq!(clock.now_unix_secs(), 999);
    }

    #[test]
    fn system_clock_is_after_2020() {
        let clock = SystemClock;
        assert!(clock.now_unix_secs() > 1_577_836_800);
    }
}
