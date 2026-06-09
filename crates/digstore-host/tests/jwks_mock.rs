use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};
use httpmock::prelude::*;

mod common;
use common::test_deps;

const CHALLENGE_LEN: usize = 32 + 32 + 8;

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
fn jwks_fetch_nosession_then_success() {
    // The SSRF guard blocks loopback/plaintext by default; the mock JWKS server
    // runs on http loopback, so opt into the documented insecure override here.
    std::env::set_var("DIGSTORE_ALLOW_INSECURE_JWKS", "1");
    let server = MockServer::start();
    let body = br#"{"keys":[]}"#;
    let mock = server.mock(|when, then| {
        when.method(GET).path("/jwks.json");
        then.status(200).body(body);
    });

    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let url = format!("{}/jwks.json", server.base_url());
    rt.write_guest(5000, url.as_bytes()).unwrap();

    // 1. NoSession before any session.
    let r0 = rt
        .call_i32_export_2("probe_jwks", 5000, url.len() as i32)
        .unwrap();
    assert_eq!(r0, -100);

    // 2. Establish a session.
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    challenge[0..32].fill(0x01);
    challenge[32..64].fill(0x02);
    challenge[64..72].copy_from_slice(&1_700_000_000u64.to_be_bytes());
    rt.write_guest(4096, &challenge).unwrap();
    assert!(rt.call_i32_export_1("probe_establish", 4096).unwrap() >= 0);

    // 3. Now the fetch succeeds and returns the body length.
    let r1 = rt
        .call_i32_export_2("probe_jwks", 5000, url.len() as i32)
        .unwrap();
    assert_eq!(r1 as usize, body.len());
    mock.assert();

    let got = rt.read_return_buffer_copy().unwrap();
    assert_eq!(&got, body);
}
