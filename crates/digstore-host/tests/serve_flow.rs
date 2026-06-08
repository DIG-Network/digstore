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

fn echo_rt() -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/serve_echo.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap()
}

#[test]
fn serve_content_round_trips_request_bytes() {
    let mut rt = echo_rt();
    let req = b"retrieval-key-and-root-and-range-bytes".to_vec();
    let out = rt.serve_content(&req).unwrap();
    assert_eq!(out, req);
}

#[test]
fn serve_proof_round_trips_request_bytes() {
    let mut rt = echo_rt();
    let req = vec![0xCDu8; 1024];
    let out = rt.serve_proof(&req).unwrap();
    assert_eq!(out, req);
}

#[test]
fn serve_content_empty_request_is_ok() {
    let mut rt = echo_rt();
    let out = rt.serve_content(&[]).unwrap();
    assert!(out.is_empty());
}
