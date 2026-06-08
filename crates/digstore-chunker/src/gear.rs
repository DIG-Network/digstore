//! The fixed gear table for the rolling content-defined hash.
//!
//! 256 distinct `u64` constants, GENERATED AT COMPILE TIME by a `const fn`
//! SplitMix64 stream. EMBEDDED and FROZEN: changing any entry changes every
//! chunk boundary in every store, so this table is part of the on-disk format
//! contract. Indices 0 and 255 are overwritten with pinned guard values for the
//! determinism guard test (`gear_table_pinned_guards_are_present`).

/// SplitMix64 seed. Changing this regenerates the entire table — do not change
/// it without re-pinning the golden vectors in `tests/vectors.rs`.
const GEAR_SEED: u64 = 0x1234_5678_9abc_def0;

/// First pinned guard value (index 0).
const GEAR_GUARD_FIRST: u64 = 0x3b5c_9f8e_2d71_a046;
/// Last pinned guard value (index 255).
const GEAR_GUARD_LAST: u64 = 0x9e1d_4a7c_60b3_82f5;

/// Build the 256-entry gear table from a SplitMix64 stream at compile time.
const fn build_gear_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut state = GEAR_SEED;
    let mut i = 0usize;
    while i < 256 {
        // SplitMix64 step.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        table[i] = z;
        i += 1;
    }
    // Pin the two guard entries for the determinism contract.
    table[0] = GEAR_GUARD_FIRST;
    table[255] = GEAR_GUARD_LAST;
    table
}

/// The fixed, frozen 256-entry gear table.
pub const GEAR_TABLE: [u64; 256] = build_gear_table();

/// One step of the gear rolling hash: shift the accumulator left by one and add
/// the gear-table value for the incoming byte. This is the FastCDC "gear"
/// recurrence. `pub(crate)` — used by `boundary.rs`, not part of the public API.
#[inline]
pub(crate) fn gear_roll(hash: u64, byte: u8) -> u64 {
    (hash << 1).wrapping_add(GEAR_TABLE[byte as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_roll_from_zero_is_the_table_entry() {
        // (0 << 1) + GEAR_TABLE[0xAB] == GEAR_TABLE[0xAB]
        assert_eq!(gear_roll(0, 0xAB), GEAR_TABLE[0xAB]);
    }

    #[test]
    fn gear_roll_shifts_then_adds() {
        // (1 << 1) + GEAR_TABLE[0] == 2 + guard_first
        let expected = 2u64.wrapping_add(GEAR_TABLE[0]);
        assert_eq!(gear_roll(1, 0), expected);
    }

    #[test]
    fn gear_roll_wraps_on_overflow() {
        // High accumulator forces wrapping in both the shift-add and the add.
        let h = u64::MAX;
        let expected = (h << 1).wrapping_add(GEAR_TABLE[0xFF]);
        assert_eq!(gear_roll(h, 0xFF), expected);
    }
}
