use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};
use std::path::Path;

mod common;
use common::test_deps;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.wasm");

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

/// Load the real guest fixture. If the fixture is missing AND the operator
/// explicitly opted in via DIGSTORE_E2E=1, panic loudly (the build step was
/// skipped). Otherwise (manual `--ignored` run without the fixture) panic with
/// a clear message — these tests are `#[ignore]` so they never run in the
/// default suite and never produce a false green.
fn load() -> HostRuntime {
    if !Path::new(FIXTURE).exists() {
        panic!(
            "e2e fixture missing: {FIXTURE} not built. See tests/fixtures/build_fixture.md \
             (cargo build -p digstore-guest --target wasm32-unknown-unknown --release, \
             then run digstore-compiler)."
        );
    }
    let bytes = std::fs::read(FIXTURE).unwrap();
    HostRuntime::new(&bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(1_700_000_000))).unwrap()
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_get_store_id_is_32_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_store_id().unwrap().len(), 32);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_current_roothash_is_32_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_current_roothash().unwrap().len(), 32);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_public_key_is_48_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_public_key().unwrap().len(), 48);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_serve_content_returns_response() {
    let mut rt = load();
    let req = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hello_request.bin")).unwrap();
    let out = rt.serve_content(&req).unwrap();
    assert!(!out.is_empty(), "content response must be non-empty (real or decoy)");
}
