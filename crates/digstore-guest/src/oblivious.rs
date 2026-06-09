//! Oblivious access (§14.3-14.4). The true number and order of chunk reads must
//! be hidden: pad the read count to a coarse bucket, then read in a per-call
//! randomized order with cover reads, re-randomized each execution via
//! `host_random_bytes`.

use alloc::vec::Vec;

/// Round `n` up to the next power-of-two bucket (min bucket = 1). Hides the true count.
pub fn padded_count(n: usize) -> usize {
    let n = n.max(1);
    n.next_power_of_two()
}

/// A per-execution access plan: the shuffled list of pool indices to read, plus
/// the slot of each real index inside `order` so the caller can recover content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPlan {
    pub order: Vec<u32>,
    pub real_positions: Vec<usize>,
}

/// Build an oblivious access plan: pad real count to a bucket, fill remaining
/// slots with deterministic cover indices drawn from `[0, pool_size)`, then
/// Fisher-Yates shuffle using bytes from `rand` (the host RNG, re-randomized per
/// call). Cover reads + shuffle hide which/how-many indices are real.
pub fn build_access_plan<F>(real: &[u32], pool_size: u32, mut rand: F) -> AccessPlan
where
    F: FnMut(u32) -> Vec<u8>,
{
    let bucket = padded_count(real.len());
    let mut slots: Vec<u32> = real.to_vec();
    // Fill cover slots with pseudo-random pool indices (distinct intent, may repeat).
    let need = bucket - slots.len();
    if need > 0 && pool_size > 0 {
        let cover_bytes = rand((need as u32) * 4);
        for i in 0..need {
            let b = i * 4;
            let v = u32::from_be_bytes([
                cover_bytes[b],
                cover_bytes[b + 1],
                cover_bytes[b + 2],
                cover_bytes[b + 3],
            ]);
            slots.push(v % pool_size);
        }
    } else {
        // even when no cover needed, consume a draw to keep RNG cadence uniform
        let _ = rand(4);
    }

    // Track where each real index currently sits, then shuffle and follow it.
    let mut positions: Vec<usize> = (0..real.len()).collect();
    let shuffle_bytes = rand((bucket as u32) * 4);
    // Fisher-Yates from the end.
    for i in (1..slots.len()).rev() {
        let b = (i % bucket) * 4;
        let r = u32::from_be_bytes([
            shuffle_bytes[b],
            shuffle_bytes[b + 1],
            shuffle_bytes[b + 2],
            shuffle_bytes[b + 3],
        ]) as usize;
        let j = r % (i + 1);
        slots.swap(i, j);
        for p in positions.iter_mut() {
            if *p == i {
                *p = j;
            } else if *p == j {
                *p = i;
            }
        }
    }
    AccessPlan {
        order: slots,
        real_positions: positions,
    }
}
