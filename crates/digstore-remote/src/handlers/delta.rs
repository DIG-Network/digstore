use crate::server::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub async fn get_delta(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response()
}
pub async fn post_delta(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response()
}
