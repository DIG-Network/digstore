use crate::backend::DeltaSet;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{DeltaChunk, DeltaNegotiateRequest, DeltaResponse, KeyTableChange};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use digstore_core::Bytes32;
use std::collections::HashMap;

fn to_wire(d: DeltaSet) -> DeltaResponse {
    DeltaResponse {
        from: d.from.to_hex(),
        to: d.to.to_hex(),
        chunks: d
            .new_chunks
            .into_iter()
            .map(|(h, data)| DeltaChunk {
                hash: h.to_hex(),
                data_b64: base64::engine::general_purpose::STANDARD.encode(data),
            })
            .collect(),
        key_table_changes: d
            .key_table_changes
            .into_iter()
            .map(|e| KeyTableChange {
                entry_b64: base64::engine::general_purpose::STANDARD.encode(e),
            })
            .collect(),
    }
}

pub async fn get_delta(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let (from, to) = match (q.get("from"), q.get("to")) {
        (Some(f), Some(t)) => match (Bytes32::from_hex(f), Bytes32::from_hex(t)) {
            (Ok(f), Ok(t)) => (f, t),
            _ => return RemoteError::BadRequest("bad from/to hex".into()).into_response(),
        },
        _ => return RemoteError::BadRequest("from and to required".into()).into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.delta(&store_id, &from, &to)).await;
    match res {
        Ok(d) => Json(to_wire(d)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn post_delta(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<DeltaNegotiateRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let to = match Bytes32::from_hex(&req.to) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad to hex".into()).into_response(),
    };
    let mut have = Vec::with_capacity(req.have.len());
    for h in &req.have {
        match Bytes32::from_hex(h) {
            Ok(v) => have.push(v),
            Err(_) => return RemoteError::BadRequest("bad have hex".into()).into_response(),
        }
    }
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.delta_from_have(&store_id, &to, &have)).await;
    match res {
        Ok(d) => Json(to_wire(d)).into_response(),
        Err(e) => e.into_response(),
    }
}
