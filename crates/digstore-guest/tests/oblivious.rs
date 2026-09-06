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
        assert!(
            padded_count(n) >= n.max(1),
            "bucket must cover true count {n}"
        );
    }
}

use digstore_guest::oblivious::build_access_plan;
use std::cell::Cell;

/// Minimal seeded RNG matching the DigHost::random_bytes counter ramp. Never
/// fails; `Rng::bytes` returns `Result` purely to match `build_access_plan`'s
/// fallible closure signature (production RNG failure is exercised separately
/// by `FailingRng`, below).
struct Rng(Cell<u32>);
impl Rng {
    fn bytes(&self, count: u32) -> Result<Vec<u8>, ()> {
        let n = self.0.get();
        self.0.set(n + 1);
        Ok((0..count)
            .map(|i| (n.wrapping_mul(97).wrapping_add(i.wrapping_mul(13))) as u8)
            .collect())
    }
}

#[test]
fn plan_includes_all_real_indices_plus_cover() {
    let real = vec![2u32, 5, 7];
    let pool_size = 32u32;
    let rng = Rng(Cell::new(0));
    let plan = build_access_plan(&real, pool_size, |c| rng.bytes(c)).unwrap();
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
        assert_eq!(
            plan.order[*pos], *idx,
            "real_positions must point at the real index"
        );
    }
}

#[test]
fn two_calls_reorder_differently() {
    let real = vec![1u32, 2, 3, 4, 5];
    let pool_size = 64u32;
    let rng_a = Rng(Cell::new(0));
    let rng_b = Rng(Cell::new(999));
    let a = build_access_plan(&real, pool_size, |c| rng_a.bytes(c)).unwrap();
    let b = build_access_plan(&real, pool_size, |c| rng_b.bytes(c)).unwrap();
    assert_ne!(
        a.order, b.order,
        "different randomness must reorder the plan"
    );
    // But both still contain all real indices.
    for r in &real {
        assert!(a.order.contains(r) && b.order.contains(r));
    }
}

// --- Regression: dig_ecosystem#2714 -----------------------------------------
// `build_access_plan` used to accept an INFALLIBLE `rand` closure, so both
// production callers (content.rs, proof.rs) could only cope with a host RNG
// error by substituting a constant (all-zero) buffer — making the shuffle/
// cover-index draw deterministic and defeating the access-pattern-hiding
// property this module exists for. `rand` is now fallible and the function
// propagates a draw failure rather than silently degrading to a constant.
//
// `FailingRng` succeeds for `succeeds` calls, then fails on every call after
// that — so tests can target EITHER of the two draw sites in
// `build_access_plan` (the cover-index draw / cadence-consuming draw, and the
// shuffle draw), not just "the RNG never works".
struct FailingRng {
    calls: Cell<u32>,
    succeeds: u32,
}
impl FailingRng {
    fn new(succeeds: u32) -> Self {
        FailingRng {
            calls: Cell::new(0),
            succeeds,
        }
    }
    fn bytes(&self, count: u32) -> Result<Vec<u8>, ()> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        if n < self.succeeds {
            Ok(vec![0xAB; count as usize])
        } else {
            Err(())
        }
    }
}

#[test]
fn first_draw_failure_is_propagated_not_swallowed() {
    // padded_count(3) = 4 > real.len(), so `need > 0` and the cover-index
    // branch draws before the shuffle draw. Failing from the very first call
    // must surface as `Err`, not an `Ok(plan)` built from a placeholder.
    let real = vec![2u32, 5, 7];
    let rng = FailingRng::new(0);
    let result = build_access_plan(&real, 32, |c| rng.bytes(c));
    assert_eq!(
        result,
        Err(()),
        "an RNG failure on the first draw must propagate as Err, never a plan built from a placeholder"
    );
}

#[test]
fn shuffle_draw_failure_is_propagated_even_after_a_successful_cover_draw() {
    // Lets the FIRST draw (cover-index fill) succeed, then fails the SECOND
    // draw (the Fisher-Yates shuffle). This is the discriminating case for a
    // fix that propagates the cover-draw's error but forgets the `?` on the
    // shuffle draw (or vice versa) -- a fixture that only ever fails on the
    // first call cannot see that mistake.
    let real = vec![2u32, 5, 7];
    let rng = FailingRng::new(1);
    let result = build_access_plan(&real, 32, |c| rng.bytes(c));
    assert_eq!(
        result,
        Err(()),
        "a shuffle-draw failure must propagate as Err even when the earlier cover draw succeeded"
    );
}
