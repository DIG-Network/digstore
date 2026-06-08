use crate::gear::gear_roll;
use digstore_core::ChunkerConfig;

/// Find the END offset (exclusive) of the chunk that begins at `start`.
///
/// FastCDC-style: roll the gear hash from `start`, but ignore boundary hits
/// until `start + min_size`. From there, cut at the first position where
/// `(hash & mask) == 0`. If no boundary is found by `start + max_size`, force a
/// cut at `start + max_size`. If `remaining <= min_size` bytes are left from
/// `start`, return `data.len()` (the trailing short chunk is permitted —
/// paper §8.1).
pub fn find_boundary(data: &[u8], start: usize, cfg: &ChunkerConfig) -> usize {
    let len = data.len();
    debug_assert!(start <= len);

    let remaining = len - start;
    // Trailing short chunk: not enough bytes left to enforce a full min_size.
    if remaining <= cfg.min_size {
        return len;
    }

    // Hard ceiling (exclusive) for this chunk's end.
    let max_end = (start + cfg.max_size).min(len);
    // First position at which a hash boundary is allowed.
    let min_end = start + cfg.min_size;

    let mut hash: u64 = 0;
    let mut i = start;
    while i < max_end {
        hash = gear_roll(hash, data[i]);
        i += 1;
        // `i` is now the exclusive end offset of the prospective chunk.
        if i >= min_end && (hash & cfg.mask) == 0 {
            return i;
        }
    }
    // No hash boundary within [min_end, max_end): forced cut.
    max_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::ChunkerConfig;

    fn cfg(min: usize, target: usize, max: usize, mask: u64) -> ChunkerConfig {
        ChunkerConfig { min_size: min, target_size: target, max_size: max, mask }
    }

    #[test]
    fn boundary_never_cuts_before_min_size() {
        // mask = 0 means (hash & 0) == 0 at EVERY position, so the first legal
        // cut is exactly at start + min_size.
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 0, &c), 100);
    }

    #[test]
    fn boundary_forces_cut_at_max_size() {
        // mask = u64::MAX means (hash & MAX) is essentially never 0, so the
        // boundary is forced at start + max_size.
        let data = vec![0xAAu8; 1000];
        let c = cfg(100, 200, 400, u64::MAX);
        assert_eq!(find_boundary(&data, 0, &c), 400);
    }

    #[test]
    fn boundary_respects_start_offset() {
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        // Chunk starting at 250: min cut at 350.
        assert_eq!(find_boundary(&data, 250, &c), 350);
    }

    #[test]
    fn boundary_returns_end_when_remainder_at_or_below_min() {
        // Only 40 bytes remain after start=960, < min_size=100:
        // return data.len() (the final short chunk).
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 960, &c), 1000);
    }

    #[test]
    fn boundary_returns_end_for_input_shorter_than_min() {
        let data = vec![0u8; 30];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 0, &c), 30);
    }

    #[test]
    fn boundary_cuts_within_bounds_on_real_hash_match() {
        // Pseudo-random bytes with a small mask (1 low bit): the cut must land in
        // [min, max] and be reproducible.
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(31) ^ 7) as u8).collect();
        let c = cfg(100, 200, 400, 0x1);
        let b = find_boundary(&data, 0, &c);
        assert!((100..=400).contains(&b), "boundary {b} must be within [min,max]");
        // Determinism: same call yields the same answer.
        assert_eq!(b, find_boundary(&data, 0, &c));
    }
}
