use digstore_core::config::HostImportsConfig;
use digstore_core::ErrorCode;
use digstore_host::{ExecutionLimits, FixedClock, HostError, HostRuntime};

mod common;
use common::{anonymous_test_deps, test_deps};

#[test]
fn missing_data_exports_report_missing_export() {
    let mut rt = probe_runtime(FixedClock::new(100));
    // import_probe.wat exports none of these; the wrappers must compile and
    // surface MissingExport rather than panicking.
    assert!(matches!(
        rt.get_public_key().unwrap_err(),
        HostError::MissingExport(_)
    ));
    assert!(matches!(
        rt.get_roothash_history().unwrap_err(),
        HostError::MissingExport(_)
    ));
    assert!(matches!(
        rt.get_metadata().unwrap_err(),
        HostError::MissingExport(_)
    ));
    assert!(matches!(
        rt.get_authentication_info().unwrap_err(),
        HostError::MissingExport(_)
    ));
}

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
    HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(clock),
    )
    .unwrap()
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

// The guest's challenge now carries the per-role attestation tag
// (SECURITY.md residual #2): ATTEST_DST || nonce(32) || store_id(32) || time_be(8).
const TAG_LEN: usize = digstore_core::ATTEST_DST.len();
const CHALLENGE_LEN: usize = TAG_LEN + 32 + 32 + 8;
const ATTESTATION_LEN: usize = 48 + 32 + 96;

fn write_challenge(rt: &mut HostRuntime, ptr: u32) {
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    challenge[..TAG_LEN].copy_from_slice(digstore_core::ATTEST_DST);
    challenge[TAG_LEN..TAG_LEN + 32].fill(0x01);
    challenge[TAG_LEN + 32..TAG_LEN + 64].fill(0x02);
    challenge[TAG_LEN + 64..TAG_LEN + 72].copy_from_slice(&1_700_000_000u64.to_be_bytes());
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

/// A host built with NO identity must answer the guest with an absence, from a
/// REAL `HostRuntime` — the arm `HostRuntime::new` selects when `identity` is
/// `None`.
///
/// Why it is asserted here and on observable behaviour rather than on the
/// backend's type: substituting
/// `BlsAttestationBackend::new(BlsSecretKey::from_seed(&[42u8; 32]), ..)` for
/// `UnavailableAttestationBackend` inside that arm reconstitutes #2553's
/// world-known key one crate below the call site, and every other test in the
/// repo stays green through it. `teehook`'s test drives the backend in
/// isolation, the guest proof test drives a double, and
/// `has_host_public_key` reads `HostKeys::bls_public`, which this arm does not
/// set. Only an attestation actually produced by an anonymous runtime can tell
/// the two apart — a substituted key SIGNS, so this test goes red.
///
/// Both halves matter: the pubkey leg catches a key handed out for free, and the
/// attest leg catches a signature produced under a key nobody controls.
#[test]
fn an_anonymous_runtime_hands_out_no_key_and_signs_nothing() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        anonymous_test_deps(FixedClock::new(1_700_000_000)),
    )
    .expect("an anonymous host still instantiates: reading consumes no identity");

    assert!(
        !rt.has_host_public_key(),
        "an anonymous runtime must install no public key"
    );

    // No key to hand out. NotFound, never 48 bytes of anything.
    let pubkey_result = rt.call_i32_export("probe_pubkey").unwrap();
    assert_eq!(
        pubkey_result,
        ErrorCode::NotFound as i32,
        "an anonymous host must report NotFound, not return key-shaped bytes"
    );

    // Nobody to speak for. The attestation must FAIL rather than be signed under
    // a stand-in key: a length here is a produced, attributable signature.
    write_challenge(&mut rt, 4096);
    let attest_result = rt.call_i32_export_1("probe_attest", 4096).unwrap();
    assert_eq!(
        attest_result,
        ErrorCode::AttestationFailed as i32,
        "an anonymous host must refuse to attest; a non-negative result means it \
         signed the challenge under some key, which it does not have"
    );
}

/// The control for the test above: the SAME runtime, varying only the identity,
/// does produce a key and an attestation. Without it, an anonymous-host test is
/// equally satisfied by a runtime that attests for nobody and by one that is
/// simply broken.
#[test]
fn the_identity_bearing_control_does_produce_a_key_and_an_attestation() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));

    assert!(rt.has_host_public_key());
    assert_eq!(rt.call_i32_export("probe_pubkey").unwrap(), 48);

    write_challenge(&mut rt, 4096);
    assert_eq!(
        rt.call_i32_export_1("probe_attest", 4096).unwrap() as usize,
        ATTESTATION_LEN
    );
}

#[test]
fn clock_advance_is_observed_by_guest() {
    let clock = FixedClock::new(1_000);
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(clock.clone()),
    )
    .unwrap();
    assert_eq!(rt.call_i64_export("probe_time").unwrap(), 1_000);
    clock.advance(500);
    assert_eq!(rt.call_i64_export("probe_time").unwrap(), 1_500);
}

#[test]
fn jwks_fetch_blocked_without_session() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let url = b"http://127.0.0.1:1/jwks.json";
    rt.write_guest(5000, url).unwrap();
    let r = rt
        .call_i32_export_2("probe_jwks", 5000, url.len() as i32)
        .unwrap();
    assert_eq!(r, -100); // ErrorCode::NoSession
}

fn probe_runtime_with_clock(clock: FixedClock) -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(clock),
    )
    .unwrap()
}

// §12.4: an expired session is reported distinctly from an absent one.
// host_verify_session returns 1 while valid and SessionExpired (-101) once
// past the TTL; jwks_fetch returns SessionExpired (-101) for a present-but-
// expired session and NoSession (-100) only when no session exists.
#[test]
fn expired_session_reports_session_expired() {
    let clock = FixedClock::new(1_700_000_000);
    let mut rt = probe_runtime_with_clock(clock.clone());

    // Absent session: verify -> 0, jwks -> NoSession (-100).
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 0);
    let url = b"http://127.0.0.1:1/jwks.json";
    rt.write_guest(5000, url).unwrap();
    assert_eq!(
        rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32)
            .unwrap(),
        -100
    );

    // Establish a session; it is now valid (TTL = 300s).
    write_challenge(&mut rt, 4096);
    assert!(rt.call_i32_export_1("probe_establish", 4096).unwrap() >= 0);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 1);

    // Advance the clock past the 300s TTL: the session now exists but is expired.
    clock.advance(301);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), -101); // SessionExpired
    assert_eq!(
        rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32)
            .unwrap(),
        -101 // SessionExpired, distinct from NoSession
    );
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
