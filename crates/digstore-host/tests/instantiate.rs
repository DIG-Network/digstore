use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

#[test]
fn instantiate_echo_and_call_get_store_id() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/echo.wat")).unwrap();
    let cfg = HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    };
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg,
        ExecutionLimits::default(),
        test_deps(FixedClock::new(1_700_000_000)),
    )
    .unwrap();

    let id = rt.get_store_id().unwrap();
    assert_eq!(id, vec![0xABu8; 32]);
}
