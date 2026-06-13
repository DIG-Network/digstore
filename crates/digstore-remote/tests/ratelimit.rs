mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use digstore_remote::{RateLimiter, RemoteServer};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn second_request_for_store_is_429_when_capacity_one() {
    let (be, _id, id_hex) = one_store();
    let rl = Arc::new(RateLimiter::new(1));
    let app = RemoteServer::with_rate_limiter(be, rl)
        .allow_anonymous()
        .router();

    let r1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);

    let r2 = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS, "(§21.8 429)");
}

#[tokio::test]
async fn requests_to_distinct_stores_not_limited_together() {
    // two stores, capacity 1 each -> each gets one OK.
    let be = Arc::new(digstore_remote::InMemoryBackend::new());
    be.add_store(b32(1), b48(2), b32(0x10), vec![0u8; 8], None);
    be.add_store(b32(3), b48(4), b32(0x20), vec![0u8; 8], None);
    let rl = Arc::new(RateLimiter::new(1));
    let app = RemoteServer::with_rate_limiter(be, rl)
        .allow_anonymous()
        .router();

    let a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{}", b32(1).to_hex()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let b = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{}", b32(3).to_hex()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(b.status(), StatusCode::OK);
}
