use crate::server::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub async fn post_proof(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response()
}
