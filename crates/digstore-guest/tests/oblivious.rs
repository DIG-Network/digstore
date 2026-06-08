use digstore_guest::oblivious::padded_count;

#[test]
fn padded_count_buckets_monotonically() {
    // Bucketing hides the true chunk count. Buckets: 1,2,4,8,16,32,... (powers of two).
    assert_eq!(padded_count(0), 1);
    assert_eq!(padded_count(1), 1);
    assert_eq!(padded_count(2), 2);
    assert_eq!(padded_count(3), 4);
    assert_eq!(padded_count(4), 4);
    assert_eq!(padded_count(5), 8);
    assert_eq!(padded_count(8), 8);
    assert_eq!(padded_count(9), 16);
}

#[test]
fn padded_count_never_below_true_count() {
    for n in 0..1000usize {
        assert!(padded_count(n) >= n.max(1), "bucket must cover true count {n}");
    }
}

use digstore_guest::oblivious::build_access_plan;
use std::cell::Cell;

/// Minimal seeded RNG matching the DigHost::random_bytes counter ramp.
struct Rng(Cell<u32>);
impl Rng {
    fn bytes(&self, count: u32) -> Vec<u8> {
        let n = self.0.get();
        self.0.set(n + 1);
        (0..count).map(|i| (n.wrapping_mul(97).wrapping_add(i.wrapping_mul(13))) as u8).collect()
    }
}

#[test]
fn plan_includes_all_real_indices_plus_cover() {
    let real = vec![2u32, 5, 7];
    let pool_size = 32u32;
    let rng = Rng(Cell::new(0));
    let plan = build_access_plan(&real, pool_size, |c| rng.bytes(c));
    // Every real index must be present.
    for r in &real {
        assert!(plan.order.contains(r), "real index {r} must be read");
    }
    // Plan length is the padded bucket (>= real count, power of two).
    assert!(plan.order.len().is_power_of_two());
    assert!(plan.order.len() >= real.len());
    // real_positions maps each real index to its slot in `order`.
    assert_eq!(plan.real_positions.len(), real.len());
    for (idx, pos) in real.iter().zip(plan.real_positions.iter()) {
        assert_eq!(plan.order[*pos], *idx, "real_positions must point at the real index");
    }
}

#[test]
fn two_calls_reorder_differently() {
    let real = vec![1u32, 2, 3, 4, 5];
    let pool_size = 64u32;
    let rng_a = Rng(Cell::new(0));
    let rng_b = Rng(Cell::new(999));
    let a = build_access_plan(&real, pool_size, |c| rng_a.bytes(c));
    let b = build_access_plan(&real, pool_size, |c| rng_b.bytes(c));
    assert_ne!(a.order, b.order, "different randomness must reorder the plan");
    // But both still contain all real indices.
    for r in &real {
        assert!(a.order.contains(r) && b.order.contains(r));
    }
}
