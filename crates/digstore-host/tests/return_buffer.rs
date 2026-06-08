use digstore_core::abi::unpack_ptr_len;
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 256 * 1024, // raise cap so we can request 128 KiB
        host_version: "dig-host-test/0.1".to_string(),
    }
}

fn rb_rt() -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/return_buffer.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap()
}

#[test]
fn small_buffer_round_trip() {
    let mut rt = rb_rt();
    let packed = rt.call_i64_export_1("fill_and_read", 100).unwrap();
    let (ptr, len) = unpack_ptr_len(packed);
    assert_eq!(len, 100);
    let bytes = rt.read_guest(ptr, len).unwrap();
    assert_eq!(bytes.len(), 100);
}

#[test]
fn buffer_grows_past_initial_capacity() {
    let mut rt = rb_rt();
    let packed = rt.call_i64_export_1("fill_and_read", 128 * 1024).unwrap();
    let (ptr, len) = unpack_ptr_len(packed);
    assert_eq!(len, 128 * 1024);
    let bytes = rt.read_guest(ptr, len).unwrap();
    assert_eq!(bytes.len(), 128 * 1024);
}
