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
    let limits = ExecutionLimits {
        timeout: Duration::from_millis(300),
        fuel: u64::MAX, // isolate: prove TIMEOUT triggers, not fuel
        ..Default::default()
    };
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let start = std::time::Instant::now();
    let err = rt.get_store_id().unwrap_err();
    assert!(matches!(err, HostError::Timeout), "expected Timeout, got {err:?}");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn fuel_exhaustion_terminates_export() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/spin.wat")).unwrap();
    let limits = ExecutionLimits {
        timeout: Duration::from_secs(30), // isolate: prove FUEL triggers, not timeout
        fuel: 1_000_000,
        ..Default::default()
    };
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let err = rt.get_store_id().unwrap_err();
    assert!(matches!(err, HostError::OutOfFuel), "expected OutOfFuel, got {err:?}");
}

#[test]
fn memory_ceiling_blocks_oversized_grow() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/grow.wat")).unwrap();
    let limits = ExecutionLimits {
        memory_bytes_max: 64 * 64 * 1024, // 64 pages = 4 MiB
        ..Default::default()
    };
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let err = rt.get_store_id().unwrap_err();
    assert!(
        matches!(err, HostError::Wasmtime(_) | HostError::MemoryLimit),
        "expected memory-limit-induced trap, got {err:?}"
    );
}

#[test]
fn memory_ceiling_allows_within_limit() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/grow.wat")).unwrap();
    let limits = ExecutionLimits::default(); // 256 pages, room for +200
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let out = rt.get_store_id().unwrap(); // grow(200) from 1 page = 201 <= 256
    assert!(out.is_empty()); // pack_ptr_len(0,0) -> empty read
}
