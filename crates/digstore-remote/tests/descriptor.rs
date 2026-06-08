mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use digstore_remote::wire::{RootHistory, StoreDescriptor};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn get_descriptor_returns_root_size_pubkey() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: StoreDescriptor = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.current_root, "10".repeat(32));
    assert_eq!(d.size, 64);
    assert_eq!(d.public_key, "02".repeat(48));
}

#[tokio::test]
async fn get_descriptor_unknown_store_is_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "99".repeat(32);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{unknown}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_roots_lists_history() {
    let (be, id, id_hex) = one_store();
    be.add_generation(&id, b32(0x10), b32(0x11), vec![0u8; 8], vec![], vec![], true);
    let app = router_for(be);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/roots"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let h: RootHistory = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(h.roots.len(), 2);
    assert_eq!(h.roots[0].root, "10".repeat(32));
    assert_eq!(h.roots[1].root, "11".repeat(32));
}
