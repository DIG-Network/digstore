use crate::backend::RemoteBackend;
use crate::error::RemoteError;
use crate::ratelimit::RateLimiter;
use axum::{
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use digstore_core::Bytes32;
use std::sync::Arc;

/// Shared server state behind an Arc; cloned into every handler.
#[derive(Clone)]
pub struct AppState {
    pub backend: Arc<dyn RemoteBackend>,
    pub rate_limiter: Arc<RateLimiter>,
}

/// The Digstore remote server. Wraps an axum Router over a RemoteBackend.
pub struct RemoteServer {
    state: AppState,
}

impl RemoteServer {
    pub fn new(backend: Arc<dyn RemoteBackend>) -> Self {
        RemoteServer {
            state: AppState {
                backend,
                rate_limiter: Arc::new(RateLimiter::new(10_000)),
            },
        }
    }

    pub fn with_rate_limiter(backend: Arc<dyn RemoteBackend>, rl: Arc<RateLimiter>) -> Self {
        RemoteServer {
            state: AppState {
                backend,
                rate_limiter: rl,
            },
        }
    }

    /// Build the axum Router exposing the full §21.2 surface.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/stores/:id", get(crate::handlers::descriptor::get_descriptor))
            .route(
                "/stores/:id/roots",
                get(crate::handlers::descriptor::get_roots),
            )
            .route(
                "/stores/:id/module",
                get(crate::handlers::module::get_module)
                    .head(crate::handlers::module::head_module)
                    .put(crate::handlers::module::put_module),
            )
            .route(
                "/stores/:id/content",
                post(crate::handlers::content::post_content),
            )
            .route("/stores/:id/proof", post(crate::handlers::proof::post_proof))
            .route(
                "/stores/:id/delta",
                get(crate::handlers::delta::get_delta).post(crate::handlers::delta::post_delta),
            )
            .with_state(self.state.clone())
    }
}

/// Parse a hex store id from a path parameter, or 400.
pub fn parse_store_id(s: &str) -> Result<Bytes32, RemoteError> {
    Bytes32::from_hex(s).map_err(|_| RemoteError::BadRequest("bad store id".into()))
}

/// Run a synchronous backend call off the async runtime (wasmtime is sync, §18).
pub async fn run_blocking<T, F>(f: F) -> Result<T, RemoteError>
where
    F: FnOnce() -> Result<T, RemoteError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| RemoteError::Internal(format!("join: {e}")))?
}

impl IntoResponse for RemoteError {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}
