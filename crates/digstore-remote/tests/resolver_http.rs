//! Integration test for [`digstore_remote::HttpHealthProbe`] against a REAL
//! `RemoteServer` over a real loopback socket — the resolver unit tests
//! (`src/resolver.rs`) cover the ladder LOGIC with a scripted probe; this file
//! proves the production `HealthProbe` impl actually talks to the wire `/health`
//! route added for the `CLAUDE.md` §5.3 client→node resolution ladder.

mod test_helpers;
use test_helpers::*;

use digstore_remote::{
    resolve_node, HealthProbe, HttpHealthProbe, OverrideInputs, RemoteServer, ResolvedTier,
};
use std::time::Duration;

async fn spawn_server() -> String {
    let (be, _id, _id_hex) = one_store();
    let app = RemoteServer::new(be).allow_anonymous().router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_health_probe_succeeds_against_a_live_server() {
    let base = spawn_server().await;
    let probe = HttpHealthProbe::default();
    assert!(probe.probe(&base, Duration::from_secs(2)).await);
}

#[tokio::test]
async fn http_health_probe_fails_against_a_closed_port() {
    // Bind + immediately drop a listener to get a port nothing is listening on.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let probe = HttpHealthProbe::default();
    assert!(
        !probe
            .probe(&format!("http://{addr}"), Duration::from_millis(300))
            .await
    );
}

#[tokio::test]
async fn http_health_probe_times_out_on_a_non_responding_host() {
    // TEST-NET-1 (RFC 5737): reserved for documentation, routers black-hole it,
    // so the connection attempt neither succeeds nor is actively refused —
    // exercising the actual timeout path (not just "connection refused").
    let probe = HttpHealthProbe::default();
    let start = std::time::Instant::now();
    let ok = probe
        .probe("http://192.0.2.1:9778", Duration::from_millis(300))
        .await;
    assert!(!ok);
    // Bounded by the timeout (with slack for scheduling), never hangs.
    assert!(start.elapsed() < Duration::from_secs(5));
}

/// End-to-end: the full ladder, with the REAL http probe, correctly picks a live
/// server standing in for `dig.local` over a dead `localhost` candidate.
#[tokio::test]
async fn full_ladder_with_real_http_probe_prefers_first_live_tier() {
    let dig_local = spawn_server().await;
    let dead_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener);
    let localhost = format!("http://{dead_addr}");

    let probe = HttpHealthProbe::default();
    let resolved = resolve_node(
        &OverrideInputs::default(),
        &dig_local,
        &localhost,
        &probe,
        Duration::from_millis(500),
    )
    .await;
    assert_eq!(resolved.base_url, dig_local);
    assert_eq!(resolved.tier, ResolvedTier::DigLocal);
}
