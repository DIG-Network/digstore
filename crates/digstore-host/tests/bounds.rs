use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostError, HostRuntime};
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

#[test]
fn timeout_terminates_runaway_export() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/spin.wat")).unwrap();
    let mut limits = ExecutionLimits::default();
    limits.timeout = Duration::from_millis(300);
    limits.fuel = u64::MAX; // isolate: prove TIMEOUT triggers, not fuel
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let start = std::time::Instant::now();
    let err = rt.get_store_id().unwrap_err();
    assert!(matches!(err, HostError::Timeout), "expected Timeout, got {err:?}");
    assert!(start.elapsed() < Duration::from_secs(3));
}
