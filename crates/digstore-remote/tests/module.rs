mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn head_module_sets_etag_and_size_no_body() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri(format!("/stores/{id_hex}/module"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let etag = resp
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(etag, format!("\"{}\"", "10".repeat(32)));
    assert_eq!(
        resp.headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap(),
        "64"
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty(), "HEAD has no body");
}

#[tokio::test]
async fn get_module_returns_wasm_bytes_with_etag() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/module"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/wasm"
    );
    assert_eq!(
        resp.headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap(),
        format!("\"{}\"", "10".repeat(32))
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.len(), 64);
}

#[tokio::test]
async fn if_none_match_current_root_returns_304() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let etag = format!("\"{}\"", "10".repeat(32));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/module"))
                .header(header::IF_NONE_MATCH, etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());
}

#[tokio::test]
async fn if_none_match_stale_root_returns_200() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let stale = format!("\"{}\"", "ff".repeat(32));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{id_hex}/module"))
                .header(header::IF_NONE_MATCH, stale)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_module_unknown_store_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "aa".repeat(32);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/stores/{unknown}/module"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
