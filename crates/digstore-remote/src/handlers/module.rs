use crate::auth::verify_push_signature;
use crate::backend::{PushMode, PushOutcome};
use crate::error::RemoteError;
use crate::etag::{etag_for_root, matches_current};
use crate::server::{parse_store_id, run_blocking, AppState};
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use digstore_core::{Bytes32, Bytes96};

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

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn parse_b32(s: &str) -> Result<Bytes32, RemoteError> {
    Bytes32::from_hex(s).map_err(|_| RemoteError::Validation("bad hex root".into()))
}

fn parse_sig(s: &str) -> Result<Bytes96, RemoteError> {
    let raw = hex::decode(s).map_err(|_| RemoteError::Validation("bad sig hex".into()))?;
    let arr: [u8; 96] = raw
        .try_into()
        .map_err(|_| RemoteError::Validation("sig must be 96 bytes".into()))?;
    Ok(Bytes96(arr))
}

/// PUT /stores/{id}/module — push (§21.4, §21.6, §21.8).
///
/// Check precedence: parse store id (400) -> store exists / load head (404)
/// -> parse parent/root/sig headers (422 on malformed) -> bearer required &
/// valid (401) -> size limit (413) -> BLS signature valid (403) -> fast-forward
/// parent == served head (409) -> accept push (201 advance / 202 pending).
pub async fn put_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();

    // 404 if store unknown.
    let head = match run_blocking({
        let b = backend.clone();
        move || b.head_state(&store_id)
    })
    .await
    {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // 422 on malformed/missing required headers.
    let (parent, root, sig) = match (
        header_str(&headers, "X-Dig-Parent"),
        header_str(&headers, "X-Dig-Root"),
        header_str(&headers, "X-Dig-Signature"),
    ) {
        (Some(p), Some(r), Some(sg)) => match (parse_b32(p), parse_b32(r), parse_sig(sg)) {
            (Ok(p), Ok(r), Ok(sg)) => (p, r, sg),
            _ => {
                return RemoteError::Validation("malformed push headers".into()).into_response()
            }
        },
        _ => return RemoteError::Validation("missing push headers".into()).into_response(),
    };

    // 401 if bearer required but missing/invalid.
    let bearer = header_str(&headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string());
    {
        let b = backend.clone();
        let sid = store_id;
        let needs = run_blocking(move || Ok(b.requires_bearer(&sid)))
            .await
            .unwrap_or(false);
        if needs {
            let b = backend.clone();
            let sid = store_id;
            let bclone = bearer.clone();
            let ok = run_blocking(move || Ok(b.check_bearer(&sid, bclone.as_deref())))
                .await
                .unwrap_or(false);
            if !ok {
                return RemoteError::MissingBearer.into_response();
            }
        }
    }

    // 413 if oversized.
    let max = run_blocking({
        let b = backend.clone();
        move || Ok(b.max_module_size())
    })
    .await
    .unwrap_or(0);
    if body.len() as u64 > max {
        return RemoteError::TooLarge(body.len() as u64).into_response();
    }

    // 403 if BLS signature invalid (CONVENTIONS C7 order: root, store_id).
    if !verify_push_signature(&head.public_key, &root, &store_id, &sig) {
        return RemoteError::Unauthorized("bad BLS signature".into()).into_response();
    }

    // 409 if not a fast-forward of the served head.
    if parent != head.served_root {
        return RemoteError::NonFastForward.into_response();
    }

    let mode = match header_str(&headers, "X-Dig-Push-Mode") {
        Some("pending") => PushMode::Pending,
        _ => PushMode::Advance,
    };

    let body_vec = body.to_vec();
    let backend = s.backend.clone();
    let res =
        run_blocking(move || backend.accept_push(&store_id, &parent, &root, &body_vec, mode)).await;
    match res {
        Ok(PushOutcome::Advanced) => {
            (StatusCode::CREATED, [(header::ETAG, etag_for_root(&root))]).into_response()
        }
        Ok(PushOutcome::Pending) => StatusCode::ACCEPTED.into_response(),
        Err(e) => e.into_response(),
    }
}
