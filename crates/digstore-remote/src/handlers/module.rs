use crate::etag::{etag_for_root, matches_current};
use crate::server::{parse_store_id, run_blocking, AppState};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

pub async fn head_module(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.head_state(&store_id)).await;
    match res {
        Ok(hs) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::ETAG, etag_for_root(&hs.served_root).parse().unwrap());
            headers.insert(
                header::CONTENT_LENGTH,
                hs.served_size.to_string().parse().unwrap(),
            );
            headers.insert(header::CONTENT_TYPE, "application/wasm".parse().unwrap());
            (StatusCode::OK, headers).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn get_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let head = match run_blocking({
        let b = backend.clone();
        move || b.head_state(&store_id)
    })
    .await
    {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // §21.7: If-None-Match equal to current root -> 304.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if matches_current(inm, &head.served_root) {
            return (
                StatusCode::NOT_MODIFIED,
                [(header::ETAG, etag_for_root(&head.served_root))],
            )
                .into_response();
        }
    }

    let res = run_blocking(move || backend.module_bytes(&store_id, None)).await;
    match res {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/wasm".to_string()),
                (header::ETAG, etag_for_root(&head.served_root)),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

// put_module remains the existing stub until Task 10.
pub async fn put_module(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response()
}
