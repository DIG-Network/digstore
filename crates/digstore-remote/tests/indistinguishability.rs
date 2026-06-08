mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use digstore_remote::wire::{ContentEnvelope, ContentRequest};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn hit_and_miss_are_indistinguishable_on_wire() {
    let (be, id, id_hex) = one_store();
    // a real hit whose ciphertext length matches a plausible decoy bucket.
    be.put_content(&id, b32(0x55), vec![7u8; 512], vec![]);
    let app_hit = router_for(be.clone());
    let app_miss = router_for(be);

    let hit_req = ContentRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
        range: None,
    };
    let miss_req = ContentRequest {
        retrieval_key: "aa".repeat(32),
        root: "10".repeat(32),
        range: None,
    };

    let rh = app_hit
        .oneshot(post_json(
            &format!("/stores/{id_hex}/content"),
            &serde_json::to_string(&hit_req).unwrap(),
        ))
        .await
        .unwrap();
    let rm = app_miss
        .oneshot(post_json(
            &format!("/stores/{id_hex}/content"),
            &serde_json::to_string(&miss_req).unwrap(),
        ))
        .await
        .unwrap();

    // same status (200), same content-type.
    assert_eq!(rh.status(), StatusCode::OK);
    assert_eq!(rm.status(), StatusCode::OK);
    assert_eq!(
        rh.headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string()),
        rm.headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string()),
    );

    // same JSON field set (envelope shape identical).
    let bh = rh.into_body().collect().await.unwrap().to_bytes();
    let bm = rm.into_body().collect().await.unwrap().to_bytes();
    let eh: ContentEnvelope = serde_json::from_slice(&bh).unwrap();
    let em: ContentEnvelope = serde_json::from_slice(&bm).unwrap();
    // both decode to the same struct shape with all three fields present.
    assert_eq!(eh.roothash, em.roothash);
    assert!(!eh.ciphertext_b64.is_empty());
    assert!(!em.ciphertext_b64.is_empty());
}
