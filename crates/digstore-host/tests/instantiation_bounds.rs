//! Instantiation is inside the sandbox budget (§18.2).
//!
//! A wasm `start` function runs during `Linker::instantiate`, before any export
//! is called. The host therefore MUST arm the fuel budget and the epoch deadline
//! on the `Store` *before* instantiating, not after: fuel consumption is enabled
//! engine-wide, so a store that has not been given fuel yet has ZERO, and any
//! wasm executed at instantiation time either traps instantly (a legitimate
//! module is rejected) or — with the guards off — runs unbounded (a hostile
//! module hangs the host). Both failure modes are covered here.

use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};
use std::time::Duration;

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

/// The truthful control: a legitimate module whose `start` does a little work
/// must instantiate AND have its start effects visible. This is what
/// distinguishes "arm the real budget before instantiating" from the nearest
/// wrong fixes — leaving the store at zero fuel, or arming a token budget.
#[test]
fn benign_start_function_instantiates_and_runs() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/start_benign.wat")).unwrap();
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(FixedClock::new(1_700_000_000)),
    )
    .expect("a module with a benign start function must instantiate");

    // `start` wrote these bytes; reading them back proves it ran to completion
    // rather than being trapped part-way through.
    assert_eq!(rt.get_store_id().unwrap(), vec![0xABu8; 32]);
}

/// The hostile counterpart: a non-terminating `start` must be cut off by the
/// EPOCH DEADLINE, and instantiation must actually return.
///
/// Two details make this test load-bearing rather than decorative:
///
/// 1. `HostRuntime::new` runs on a worker thread behind `recv_timeout`. Timing
///    the call inline cannot detect the failure it names — if instantiation
///    never returns, the timing assertion that follows it is unreachable and the
///    test hangs instead of failing.
/// 2. Fuel is set to `u64::MAX` to ISOLATE the epoch deadline (the discipline in
///    `tests/bounds.rs`). With a finite fuel budget the spin would trip
///    `OutOfFuel` first and the epoch deadline would never be exercised, so a
///    regression that dropped `set_epoch_deadline` would stay green.
#[test]
fn runaway_start_function_is_cut_off_by_the_epoch_deadline() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/start_spin.wat")).unwrap();
    let limits = ExecutionLimits {
        timeout: Duration::from_millis(300),
        fuel: u64::MAX, // isolate: prove the EPOCH deadline triggers, not fuel
        ..Default::default()
    };

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = HostRuntime::new(
            &module_bytes,
            cfg(),
            limits,
            test_deps(FixedClock::new(1_700_000_000)),
        )
        .map(|_| ())
        .map_err(|e| format!("{e:?}"));
        // A closed receiver just means the test already gave up; nothing to do.
        let _ = tx.send(outcome);
    });

    // Generous next to the 300 ms deadline, but finite: a hang FAILS here rather
    // than blocking the suite forever.
    let result = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("instantiation of a non-terminating start function never returned");

    let err = result.expect_err("a non-terminating start function must be rejected");
    assert!(
        err.contains("Timeout"),
        "expected the epoch deadline (HostError::Timeout), got {err}"
    );
}
