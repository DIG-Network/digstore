//! Per-request authentication (paper §21.9) enforcement on the runnable server.
//!
//! A `RemoteServer::new(...)` (the default the binary uses) REQUIRES every request
//! to carry a fresh, valid signature from the caller's identity key. These tests
//! prove: no headers → 401; a valid signature → the request reaches the handler;
//! a wrong-method signature (replay) → 401.

mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use digstore_core::Bytes32;
use digstore_remote::RemoteServer;
use tower::ServiceExt;

/// Build the §21.9 auth headers for a GET descriptor request ("fetch").
fn auth_headers_for(
    builder: axum::http::request::Builder,
    sk: &digstore_crypto::bls::SecretKey,
    pk_hex: &str,
    method_tag: &str,
    store: &Bytes32,
    ts: u64,
) -> Request<Body> {
    let nonce = [7u8; 32];
    let msg = digstore_crypto::request_signing_message(method_tag, store, ts, &nonce);
    let sig = digstore_crypto::bls_sign(sk, &msg);
    builder
        .header("X-Dig-Identity", pk_hex)
        .header("X-Dig-Timestamp", ts.to_string())
        .header("X-Dig-Nonce", hex::encode(nonce))
        .header("X-Dig-Auth", hex::encode(sig.0))
        .body(Body::empty())
        .unwrap()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn unauthenticated_request_is_401() {
    let (be, _id, id_hex) = one_store();
    let app = RemoteServer::new(be).router(); // auth REQUIRED (default)
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/stores/{id_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn valid_identity_signature_passes_auth() {
    let (be, id, id_hex) = one_store();
    let (sk, pk) = digstore_crypto::bls_keygen(&[99u8; 32]);
    let app = RemoteServer::new(be).router();
    let req = auth_headers_for(
        Request::builder()
            .method(Method::GET)
            .uri(format!("/stores/{id_hex}")),
        &sk,
        &pk.to_hex(),
        "fetch",
        &id,
        now(),
    );
    let resp = app.oneshot(req).await.unwrap();
    // Auth passed → the descriptor handler ran and returned 200 (not 401).
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn wrong_method_signature_is_rejected() {
    // A signature minted for "push" must not authenticate a "fetch" (GET descriptor).
    let (be, id, id_hex) = one_store();
    let (sk, pk) = digstore_crypto::bls_keygen(&[99u8; 32]);
    let app = RemoteServer::new(be).router();
    let req = auth_headers_for(
        Request::builder()
            .method(Method::GET)
            .uri(format!("/stores/{id_hex}")),
        &sk,
        &pk.to_hex(),
        "push", // wrong tag for a GET descriptor
        &id,
        now(),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_timestamp_is_rejected() {
    let (be, id, id_hex) = one_store();
    let (sk, pk) = digstore_crypto::bls_keygen(&[99u8; 32]);
    let app = RemoteServer::new(be).router();
    let req = auth_headers_for(
        Request::builder()
            .method(Method::GET)
            .uri(format!("/stores/{id_hex}")),
        &sk,
        &pk.to_hex(),
        "fetch",
        &id,
        now() - 10_000, // well outside the freshness window
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
