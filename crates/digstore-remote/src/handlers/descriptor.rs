use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{RootEntry, RootHistory, StoreDescriptor};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

pub async fn get_descriptor(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.head_state(&store_id)).await;
    match res {
        Ok(hs) => {
            let body = StoreDescriptor {
                current_root: hs.served_root.to_hex(),
                size: hs.served_size,
                public_key: hs.public_key.to_hex(),
            };
            Json(body).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn get_roots(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.root_history(&store_id)).await;
    match res {
        Ok(records) => {
            let roots = records
                .into_iter()
                .map(|r| RootEntry {
                    generation: r.generation,
                    root: r.root.to_hex(),
                    timestamp: r.timestamp,
                })
                .collect();
            Json(RootHistory { roots }).into_response()
        }
        Err(e) => e.into_response(),
    }
}
