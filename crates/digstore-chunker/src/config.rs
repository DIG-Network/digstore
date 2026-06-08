use digstore_core::ChunkerConfig;

/// Derive the FastCDC boundary mask from the target chunk size.
///
/// The mask is `2^floor(log2(target_size)) - 1`, i.e. it has
/// `floor(log2(target_size))` low bits set. A boundary is declared when
/// `(hash & mask) == 0`, which occurs with probability `2^-bits` per byte
/// position, giving an expected chunk length of `target_size` bytes (paper §8.1).
pub fn mask_for_target(target_size: usize) -> u64 {
    if target_size < 2 {
        return 0;
    }
    // floor(log2(target_size)) = bit index of the highest set bit.
    let bits = (usize::BITS - 1 - target_size.leading_zeros()) as u64;
    if bits == 0 {
        0
    } else if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The canonical Digstore chunker configuration:
/// min 16 KiB, target 64 KiB, max 256 KiB, mask derived from target.
pub fn default_config() -> ChunkerConfig {
    let target_size = 64 * 1024;
    ChunkerConfig {
        min_size: 16 * 1024,
        target_size,
        max_size: 256 * 1024,
        mask: mask_for_target(target_size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_for_64kib_target_has_16_low_bits() {
        assert_eq!(mask_for_target(64 * 1024), 0xFFFF);
    }

    #[test]
    fn mask_for_target_is_power_of_two_minus_one() {
        let m = mask_for_target(64 * 1024);
        assert_eq!(m & (m + 1), 0, "mask must be 2^k - 1");
    }

    #[test]
    fn mask_for_non_power_of_two_uses_floor_log2() {
        // floor(log2(100_000)) = 16 -> 0xFFFF
        assert_eq!(mask_for_target(100_000), 0xFFFF);
        // floor(log2(70_000)) = 16 -> 0xFFFF
        assert_eq!(mask_for_target(70_000), 0xFFFF);
        // 32 KiB = 2^15 -> 0x7FFF
        assert_eq!(mask_for_target(32 * 1024), 0x7FFF);
    }

    #[test]
    fn mask_for_zero_or_tiny_target_is_minimal() {
        assert_eq!(mask_for_target(0), 0);
        assert_eq!(mask_for_target(1), 0); // floor(log2(1)) = 0 -> mask 0
    }

    #[test]
    fn default_config_matches_canonical_bounds() {
        let c = default_config();
        assert_eq!(c.min_size, 16 * 1024);
        assert_eq!(c.target_size, 64 * 1024);
        assert_eq!(c.max_size, 256 * 1024);
        assert_eq!(c.mask, 0xFFFF);
    }
}
