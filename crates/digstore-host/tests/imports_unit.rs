use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

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

fn probe_runtime(clock: FixedClock) -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(clock)).unwrap()
}

#[test]
fn host_time_returns_injected_clock() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let t = rt.call_i64_export("probe_time").unwrap();
    assert_eq!(t, 1_700_000_000);
}

#[test]
fn host_random_under_cap_writes_buffer() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 64).unwrap();
    assert_eq!(n, 64);
}

#[test]
fn host_random_over_cap_errors() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 2048).unwrap();
    assert!(n < 0);
}

#[test]
fn host_public_key_returns_48_bytes() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export("probe_pubkey").unwrap();
    assert_eq!(n, 48);
}

#[test]
fn read_return_buffer_copies_into_guest() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 16).unwrap();
    assert_eq!(n, 16);
    let copied = rt.call_i32_export_1("probe_read", 2048).unwrap();
    assert_eq!(copied, 16);
    let mem = rt.read_guest(2048, 16).unwrap();
    assert_eq!(mem.len(), 16);
}
