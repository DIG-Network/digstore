//! CSPRNG backing `host_random_bytes` (§12). Capped at `max_random_bytes`.
//! Seedable so the oblivious-access cover reads (§14.3) are reproducible in tests.

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;

pub struct HostRng {
    inner: ChaCha20Rng,
}

impl HostRng {
    /// Production constructor seeded from OS entropy.
    pub fn from_entropy() -> Self {
        HostRng {
            inner: ChaCha20Rng::from_entropy(),
        }
    }

    /// Deterministic constructor for tests.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        HostRng {
            inner: ChaCha20Rng::from_seed(seed),
        }
    }

    /// Produce `count` random bytes, or `None` if `count > max` (cap enforced).
    pub fn fill(&mut self, count: usize, max: usize) -> Option<Vec<u8>> {
        if count > max {
            return None;
        }
        let mut buf = vec![0u8; count];
        self.inner.fill_bytes(&mut buf);
        Some(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_returns_requested_count_under_cap() {
        let mut rng = HostRng::from_seed([7u8; 32]);
        let out = rng.fill(64, 1024).unwrap();
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn fill_over_cap_is_rejected() {
        let mut rng = HostRng::from_seed([7u8; 32]);
        assert!(rng.fill(2048, 1024).is_none());
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        let mut a = HostRng::from_seed([1u8; 32]);
        let mut b = HostRng::from_seed([1u8; 32]);
        assert_eq!(a.fill(32, 1024), b.fill(32, 1024));
    }

    #[test]
    fn distinct_seeds_differ() {
        let mut a = HostRng::from_seed([1u8; 32]);
        let mut b = HostRng::from_seed([2u8; 32]);
        assert_ne!(a.fill(32, 1024), b.fill(32, 1024));
    }
}
