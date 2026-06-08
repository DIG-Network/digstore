use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of wall-clock time. Injected into `Store` so commits are deterministic
/// in tests. Generation timestamps (unix seconds) come from this.
pub trait Clock {
    /// Current time in unix seconds.
    fn unix_seconds(&self) -> u64;
}

/// Real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs()
    }
}

/// Deterministic clock for tests; can be advanced explicitly.
pub struct FixedClock {
    now: Cell<u64>,
}

impl FixedClock {
    pub fn new(now: u64) -> Self {
        Self {
            now: Cell::new(now),
        }
    }
    /// Move the clock forward by `delta` seconds.
    pub fn advance(&self, delta: u64) {
        self.now.set(self.now.get() + delta);
    }
}

impl Clock for FixedClock {
    fn unix_seconds(&self) -> u64 {
        self.now.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_fixed_value() {
        let c = FixedClock::new(1_717_000_000);
        assert_eq!(c.unix_seconds(), 1_717_000_000);
        assert_eq!(c.unix_seconds(), 1_717_000_000);
    }

    #[test]
    fn fixed_clock_can_advance() {
        let c = FixedClock::new(100);
        c.advance(50);
        assert_eq!(c.unix_seconds(), 150);
    }

    #[test]
    fn system_clock_is_nonzero() {
        let c = SystemClock;
        assert!(c.unix_seconds() > 1_600_000_000);
    }
}
