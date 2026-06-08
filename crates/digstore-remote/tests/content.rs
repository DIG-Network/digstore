mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use digstore_remote::wire::{ContentEnvelope, ContentRequest, ProofEnvelope, ProofRequest};
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
async fn content_hit_returns_real_ciphertext_200() {
    let (be, id, id_hex) = one_store();
    be.put_content(&id, b32(0x55), vec![10, 20, 30], vec![1, 1]);
    let app = router_for(be);
    let req = ContentRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
        range: None,
    };
    let resp = app
        .oneshot(post_json(
            &format!("/stores/{id_hex}/content"),
            &serde_json::to_string(&req).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(env.ciphertext_b64)
        .unwrap();
    assert_eq!(ct, vec![10, 20, 30]);
    assert_eq!(env.roothash, "10".repeat(32));
}

#[tokio::test]
async fn content_miss_returns_200_decoy_never_404() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let req = ContentRequest {
        retrieval_key: "ab".repeat(32),
        root: "10".repeat(32),
        range: None,
    };
    let resp = app
        .oneshot(post_json(
            &format!("/stores/{id_hex}/content"),
            &serde_json::to_string(&req).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "content miss must be 200 decoy, never 404 (§21.8/§14.2)"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(env.ciphertext_b64)
        .unwrap();
    assert!(!ct.is_empty(), "decoy has real-looking bytes");
}

#[tokio::test]
async fn content_miss_is_deterministic_same_key_same_bytes() {
    let (be, _id, id_hex) = one_store();
    let app1 = router_for(be.clone());
    let app2 = router_for(be);
    let req = ContentRequest {
        retrieval_key: "cc".repeat(32),
        root: "10".repeat(32),
        range: None,
    };
    let body = serde_json::to_string(&req).unwrap();
    let r1 = app1
        .oneshot(post_json(&format!("/stores/{id_hex}/content"), &body))
        .await
        .unwrap();
    let r2 = app2
        .oneshot(post_json(&format!("/stores/{id_hex}/content"), &body))
        .await
        .unwrap();
    let b1 = r1.into_body().collect().await.unwrap().to_bytes();
    let b2 = r2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(b1, b2, "same miss -> identical decoy (§14.2)");
}

#[tokio::test]
async fn content_unknown_store_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "99".repeat(32);
    let req = ContentRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
        range: None,
    };
    let resp = app
        .oneshot(post_json(
            &format!("/stores/{unknown}/content"),
            &serde_json::to_string(&req).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "unknown store IS 404 (only content miss is exempt)"
    );
}

#[tokio::test]
async fn content_range_slices_ciphertext() {
    let (be, id, id_hex) = one_store();
    be.put_content(&id, b32(0x55), vec![0, 1, 2, 3, 4, 5, 6, 7], vec![]);
    let app = router_for(be);
    let req = ContentRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
        range: Some(digstore_remote::wire::ByteRange { start: 2, end: 5 }),
    };
    let resp = app
        .oneshot(post_json(
            &format!("/stores/{id_hex}/content"),
            &serde_json::to_string(&req).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(env.ciphertext_b64)
        .unwrap();
    assert_eq!(ct, vec![2, 3, 4]);
}

#[tokio::test]
async fn proof_returns_200_with_roothash() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let req = ProofRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
    };
    let resp = app
        .oneshot(post_json(
            &format!("/stores/{id_hex}/proof"),
            &serde_json::to_string(&req).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ProofEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(env.roothash, "10".repeat(32));
}
