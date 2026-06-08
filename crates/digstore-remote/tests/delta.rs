mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use digstore_core::Bytes32;
use digstore_remote::wire::{DeltaNegotiateRequest, DeltaResponse};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// store with genesis 0x10 (0 chunks) and child 0x40 (adds A2, A3) advanced.
fn store_with_two_gens() -> (std::sync::Arc<digstore_remote::InMemoryBackend>, Bytes32, String) {
    let (be, id, id_hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x40),
        vec![0u8; 16],
        vec![(b32(0xA2), vec![2, 2]), (b32(0xA3), vec![3, 3, 3])],
        vec![vec![9, 9]],
        true,
    );
    (be, id, id_hex)
}

#[tokio::test]
async fn get_delta_returns_only_new_chunks_and_keytable_changes() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let from = "10".repeat(32);
    let to = "40".repeat(32);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/delta?from={from}&to={to}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: DeltaResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.from, from);
    assert_eq!(d.to, to);
    assert_eq!(d.chunks.len(), 2, "exactly the new chunks");
    assert_eq!(d.key_table_changes.len(), 1);
}

#[tokio::test]
async fn get_delta_missing_query_is_400() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/delta"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_delta_unknown_root_is_404() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let from = "10".repeat(32);
    let to = "ee".repeat(32);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/delta?from={from}&to={to}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_delta_negotiates_from_have_summary() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    // client already holds A2 -> server returns only A3.
    let req = DeltaNegotiateRequest {
        to: "40".repeat(32),
        have: vec!["a2".repeat(32)],
    };
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/stores/{id_hex}/delta"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_string(&req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: DeltaResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.chunks.len(), 1, "client had A2, only A3 returned");
    assert_eq!(d.chunks[0].hash, "a3".repeat(32));
}
