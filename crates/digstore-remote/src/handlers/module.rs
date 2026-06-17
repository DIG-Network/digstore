use crate::auth::verify_push_signature;
use crate::backend::{PushMode, PushOutcome};
use crate::error::RemoteError;
use crate::etag::{etag_for_root, matches_current};
use crate::server::{parse_store_id, run_blocking, AppState};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use digstore_core::{Bytes32, Bytes96};
use std::collections::HashMap;

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
            headers.insert(
                header::ETAG,
                etag_for_root(&hs.served_root).parse().unwrap(),
            );
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

/// POST /stores/{id}/module/upload — push-init (dig RPC push protocol v1). A self-hosted node has
/// no object store to presign against, so it always negotiates INLINE: the caller then PUTs the
/// body to /module?root=. The publisher push signature (C7) + fast-forward are verified up front so
/// a bad push is rejected before any bytes are uploaded.
pub async fn post_upload_init(
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
    let head = match run_blocking({
        let b = backend.clone();
        move || b.head_state(&store_id)
    })
    .await
    {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };
    let init: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return RemoteError::Validation("bad push-init body".into()).into_response(),
    };
    let new_root = match init.get("new_root").and_then(|v| v.as_str()).map(parse_b32) {
        Some(Ok(r)) => r,
        _ => return RemoteError::Validation("bad new_root".into()).into_response(),
    };
    let sig = match header_str(&headers, "X-Dig-Signature").map(parse_sig) {
        Some(Ok(sg)) => sg,
        _ => {
            return RemoteError::Validation("missing/malformed X-Dig-Signature".into())
                .into_response()
        }
    };
    // Ownership: the publisher push signature (CONVENTIONS C7) must verify against the store key.
    if !verify_push_signature(&head.public_key, &new_root, &store_id, &sig) {
        return RemoteError::Unauthorized("bad BLS signature".into()).into_response();
    }
    // Fast-forward: the declared parent must equal the served head. An EMPTY parent_root means
    // "first push — fast-forward from genesis (no head yet)" (the client sends it empty rather than
    // the all-zero root); accept it iff the served head is genesis. This mirrors the rpc.dig.net
    // retrieval's empty-parent convention so a first push to a self-hosted `digstore serve` node
    // (and the integration tests) is not rejected with a spurious non-fast-forward.
    let parent = init
        .get("parent_root")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let parent_ok = if parent.is_empty() {
        head.served_root == Bytes32::default()
    } else {
        matches!(parse_b32(parent), Ok(p) if p == head.served_root)
    };
    if !parent_ok {
        return RemoteError::NonFastForward.into_response();
    }
    let out = serde_json::json!({ "mode": "inline", "upload_id": new_root.to_hex() });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        out.to_string(),
    )
        .into_response()
}

/// POST /stores/{id}/module/complete — only meaningful for a PRESIGNED upload. A self-hosted node
/// never offers presigned mode (it accepts the body inline via PUT /module), so there is nothing to
/// complete here.
pub async fn post_complete(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    RemoteError::Validation(
        "this node serves inline uploads; PUT the module to /module?root= instead".into(),
    )
    .into_response()
}

/// PUT /stores/{id}/module?root= — push finalize (§21.4, §21.6, §21.8). The inline-mode upload
/// leg of push protocol v1 (and the legacy single-PUT). Check precedence: parse store id (400) ->
/// store exists / load head (404) -> root from ?root= or X-Dig-Root, sig from X-Dig-Signature (422
/// on malformed) -> bearer required & valid (401) -> size limit (413) -> BLS signature valid (403)
/// -> fast-forward parent == served head (409) -> accept push (201 advance / 202 pending).
pub async fn put_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
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

    // root: ?root= (push protocol v1) or the X-Dig-Root header (legacy single-PUT). 422 if neither.
    let root = match q
        .get("root")
        .map(String::as_str)
        .or_else(|| header_str(&headers, "X-Dig-Root"))
    {
        Some(r) => match parse_b32(r) {
            Ok(r) => r,
            Err(_) => return RemoteError::Validation("malformed root".into()).into_response(),
        },
        None => return RemoteError::Validation("missing root".into()).into_response(),
    };
    // parent: the X-Dig-Parent header (legacy), else the served head (push protocol v1 derives it —
    // the init step already validated the caller's declared parent against this head).
    let parent = match header_str(&headers, "X-Dig-Parent") {
        Some(p) => match parse_b32(p) {
            Ok(p) => p,
            Err(_) => return RemoteError::Validation("malformed parent".into()).into_response(),
        },
        None => head.served_root,
    };
    // sig: X-Dig-Signature (required). 422 if missing/malformed.
    let sig = match header_str(&headers, "X-Dig-Signature").map(parse_sig) {
        Some(Ok(sg)) => sg,
        _ => return RemoteError::Validation("missing/malformed signature".into()).into_response(),
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
    // `sig` was verified at the BLS check above; persist it so a later clone/pull
    // can re-verify head authorization (§21.6).
    let res = run_blocking(move || {
        backend.accept_push(&store_id, &parent, &root, &body_vec, Some(&sig), mode)
    })
    .await;
    match res {
        Ok(PushOutcome::Advanced) => {
            (StatusCode::CREATED, [(header::ETAG, etag_for_root(&root))]).into_response()
        }
        Ok(PushOutcome::Pending) => StatusCode::ACCEPTED.into_response(),
        Err(e) => e.into_response(),
    }
}
