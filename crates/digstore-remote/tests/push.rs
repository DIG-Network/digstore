mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use digstore_core::{Bytes32, Bytes96};
use digstore_remote::{auth::push_signing_message, InMemoryBackend, RemoteBackend, RemoteServer};
use std::sync::Arc;
use tower::ServiceExt;

/// Build a store whose public key is a real BLS key; return (backend, id, id_hex, secret_key).
fn signed_store() -> (
    Arc<InMemoryBackend>,
    Bytes32,
    String,
    digstore_crypto::bls::SecretKey,
) {
    let (sk, pk) = digstore_crypto::bls_keygen(&[42u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(1);
    be.add_store(id, pk, b32(0x10), vec![0u8; 64], None);
    (be, id, id.to_hex(), sk)
}

#[allow(clippy::too_many_arguments)]
fn put_req(
    id_hex: &str,
    parent: &str,
    root: &str,
    sig_hex: &str,
    mode: Option<&str>,
    bearer: Option<&str>,
    body: Vec<u8>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::PUT)
        .uri(format!("/stores/{id_hex}/module"))
        .header("X-Dig-Parent", parent)
        .header("X-Dig-Root", root)
        .header("X-Dig-Signature", sig_hex);
    if let Some(m) = mode {
        b = b.header("X-Dig-Push-Mode", m);
    }
    if let Some(t) = bearer {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn valid_push_advances_head_201() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    // CONVENTIONS C7: message order is (root, store_id).
    let msg = push_signing_message(&new_root, &id);
    let sig = digstore_crypto::bls_sign(&sk, &msg);
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("advance"),
            None,
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let hs = be.head_state(&id).unwrap();
    assert_eq!(hs.served_root, new_root, "served head advanced");
    assert_eq!(hs.served_size, 80);
}

#[tokio::test]
async fn bad_signature_is_403() {
    let (be, id, id_hex, _sk) = signed_store();
    let new_root = b32(0x20);
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    let bad_sig = Bytes96([0xCD; 96]);
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(bad_sig.0),
            Some("advance"),
            None,
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert!(resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::UNAUTHORIZED);
    assert_eq!(
        be.head_state(&id).unwrap().served_root,
        b32(0x10),
        "head not advanced on bad sig"
    );
}

#[tokio::test]
async fn missing_bearer_when_required_is_401() {
    let (be, id, id_hex, sk) = signed_store();
    be.set_bearer(&id, "tok");
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&new_root, &id));
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    // valid sig but NO bearer header -> 401
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("advance"),
            None,
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn correct_bearer_and_sig_is_201() {
    let (be, id, id_hex, sk) = signed_store();
    be.set_bearer(&id, "tok");
    let new_root = b32(0x21);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&new_root, &id));
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("advance"),
            Some("tok"),
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn non_fast_forward_is_409() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&new_root, &id));
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    // parent does NOT match served head 0x10 -> 409
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"ee".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("advance"),
            None,
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(be.head_state(&id).unwrap().served_root, b32(0x10));
}

#[tokio::test]
async fn pending_push_is_202_and_served_head_unchanged() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&new_root, &id));
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("pending"),
            None,
            vec![1u8; 80],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let hs = be.head_state(&id).unwrap();
    assert_eq!(
        hs.served_root,
        b32(0x10),
        "served head unchanged on pending (§21.4)"
    );
    assert_eq!(hs.pending_root, Some(new_root));
}

#[tokio::test]
async fn oversized_module_is_413() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[7u8; 32]);
    let be = Arc::new(InMemoryBackend::with_max_module_size(16));
    let id = b32(1);
    be.add_store(id, pk, b32(0x10), vec![0u8; 8], None);
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&new_root, &id));
    let app = RemoteServer::new(be.clone()).allow_anonymous().router();
    let resp = app
        .oneshot(put_req(
            &id.to_hex(),
            &"10".repeat(32),
            &new_root.to_hex(),
            &hex::encode(sig.0),
            Some("advance"),
            None,
            vec![1u8; 100],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn malformed_signature_header_is_422_or_403() {
    let (be, _id, id_hex, _sk) = signed_store();
    let app = RemoteServer::new(be).allow_anonymous().router();
    let resp = app
        .oneshot(put_req(
            &id_hex,
            &"10".repeat(32),
            &"20".repeat(32),
            "not-hex",
            Some("advance"),
            None,
            vec![1u8; 8],
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::UNPROCESSABLE_ENTITY || resp.status() == StatusCode::FORBIDDEN
    );
}
