//! Execution bounds (§18.2): wall-clock timeout, outer memory ceiling, fuel.

use std::time::Duration;

/// Page size of WASM linear memory.
pub const WASM_PAGE_SIZE: usize = 64 * 1024;

/// Hard ceiling matching the guest's declared max (256 pages = 16 MiB, §18.2).
pub const MAX_MEMORY_BYTES: usize = 256 * WASM_PAGE_SIZE;

#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    /// Wall-clock budget for a single export call (enforced via epoch interruption).
    pub timeout: Duration,
    /// Outer linear-memory ceiling in bytes (StoreLimits, §18.2).
    pub memory_bytes_max: usize,
    /// Fuel budget for a single export call.
    pub fuel: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        ExecutionLimits {
            timeout: Duration::from_secs(5),
            memory_bytes_max: MAX_MEMORY_BYTES,
            fuel: 5_000_000_000,
        }
    }
}

impl ExecutionLimits {
    pub fn memory_pages_max(&self) -> usize {
        self.memory_bytes_max / WASM_PAGE_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn defaults_match_spec() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_bytes_max, 16 * 1024 * 1024);
        assert_eq!(l.timeout, Duration::from_secs(5));
        assert!(l.fuel >= 1_000_000_000);
    }

    #[test]
    fn pages_helper_matches_bytes() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_pages_max(), 256);
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(WASM_PAGE_SIZE, 64 * 1024);
        assert_eq!(MAX_MEMORY_BYTES, 16 * 1024 * 1024);
    }
}
