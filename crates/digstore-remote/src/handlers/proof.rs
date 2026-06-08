use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{ProofEnvelope, ProofRequest};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use digstore_core::Bytes32;

pub async fn post_proof(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ProofRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let retrieval_key = match Bytes32::from_hex(&req.retrieval_key) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad retrieval key".into()).into_response(),
    };
    let root = match Bytes32::from_hex(&req.root) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad root".into()).into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.serve_proof(&store_id, &retrieval_key, &root)).await;
    match res {
        Ok((proof, roothash)) => {
            let env = ProofEnvelope {
                proof_b64: base64::engine::general_purpose::STANDARD.encode(proof),
                roothash: roothash.to_hex(),
            };
            Json(env).into_response()
        }
        Err(e) => e.into_response(),
    }
}
