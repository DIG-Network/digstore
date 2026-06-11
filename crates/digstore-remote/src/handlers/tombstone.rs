//! Accept a signed revocation tombstone (SECURITY.md residual #1 Layer 1).
//!
//! `POST /stores/{id}/tombstone` carries a canonical `Tombstone` record + the
//! publisher's BLS signature. The remote VERIFIES the signature against the
//! store's published key BEFORE persisting it (fail-closed on a bad/wrong-key
//! signature), mirroring how `put_module` verifies the push signature before
//! `accept_push`. A stored tombstone is then served in the store descriptor and
//! re-verified by clients on clone/pull.

use crate::backend::StoredTombstone;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::TombstoneRequest;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use digstore_core::{Bytes96, Decode, Tombstone};

fn parse_sig(s: &str) -> Result<Bytes96, RemoteError> {
    let raw =
        hex::decode(s).map_err(|_| RemoteError::Validation("bad tombstone sig hex".into()))?;
    let arr: [u8; 96] = raw
        .try_into()
        .map_err(|_| RemoteError::Validation("tombstone sig must be 96 bytes".into()))?;
    Ok(Bytes96(arr))
}

/// POST /stores/{id}/tombstone — accept a signed revocation tombstone.
///
/// Check precedence: parse store id (400) -> store exists / load head (404) ->
/// parse record/sig (422 on malformed) -> tombstone store_id matches the path id
/// (422) -> BLS signature valid against the store's published key (403) ->
/// persist (201).
pub async fn post_tombstone(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TombstoneRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();

    // 404 if store unknown (also gives us the store's published key).
    let head = match run_blocking({
        let b = backend.clone();
        move || b.head_state(&store_id)
    })
    .await
    {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // 422 on malformed record/signature.
    let record = match hex::decode(&req.record) {
        Ok(r) => r,
        Err(_) => {
            return RemoteError::Validation("bad tombstone record hex".into()).into_response()
        }
    };
    let tombstone = match Tombstone::from_bytes(&record) {
        Ok(t) => t,
        Err(_) => {
            return RemoteError::Validation("malformed tombstone record".into()).into_response()
        }
    };
    let sig = match parse_sig(&req.signature) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    // 422 if the tombstone is bound to a different store than the path id: a
    // tombstone signed for store A must not be accepted under store B's id.
    if tombstone.store_id != store_id {
        return RemoteError::Validation("tombstone store_id does not match path".into())
            .into_response();
    }

    // 403 if the signature does not verify against the store's published key.
    // Fail-closed: a bad or wrong-key tombstone is rejected, never stored.
    let pk = match digstore_crypto::bls::PublicKey::from_bytes(&head.public_key) {
        Ok(p) => p,
        Err(_) => {
            return RemoteError::Validation("store has no valid public key".into()).into_response()
        }
    };
    if !digstore_crypto::verify_tombstone(&pk, &tombstone, &sig) {
        return RemoteError::Unauthorized("tombstone signature does not verify".into())
            .into_response();
    }

    let stored = StoredTombstone {
        tombstone,
        signature: sig,
    };
    let backend = s.backend.clone();
    match run_blocking(move || backend.store_tombstone(&store_id, &stored)).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => e.into_response(),
    }
}
