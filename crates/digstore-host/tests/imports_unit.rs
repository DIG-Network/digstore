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

const CHALLENGE_LEN: usize = 32 + 32 + 8;
const ATTESTATION_LEN: usize = 48 + 32 + 96;

fn write_challenge(rt: &mut HostRuntime, ptr: u32) {
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    challenge[0..32].fill(0x01);
    challenge[32..64].fill(0x02);
    challenge[64..72].copy_from_slice(&1_700_000_000u64.to_be_bytes());
    rt.write_guest(ptr, &challenge).unwrap();
}

#[test]
fn create_attestation_writes_response() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    write_challenge(&mut rt, 4096);
    let n = rt.call_i32_export_1("probe_attest", 4096).unwrap();
    assert_eq!(n as usize, ATTESTATION_LEN);
    let resp = rt.read_return_buffer_copy().unwrap();
    assert_eq!(resp.len(), ATTESTATION_LEN);
}

#[test]
fn establish_then_verify_session() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    write_challenge(&mut rt, 4096);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 0);
    let r = rt.call_i32_export_1("probe_establish", 4096).unwrap();
    assert!(r >= 0);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 1);
}

#[test]
fn host_public_key_returns_48_bytes() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export("probe_pubkey").unwrap();
    assert_eq!(n, 48);
}

#[test]
fn jwks_fetch_blocked_without_session() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let url = b"http://127.0.0.1:1/jwks.json";
    rt.write_guest(5000, url).unwrap();
    let r = rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32).unwrap();
    assert_eq!(r, -100); // ErrorCode::NoSession
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
