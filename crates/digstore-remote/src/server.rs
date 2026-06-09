use crate::backend::RemoteBackend;
use crate::error::RemoteError;
use crate::ratelimit::RateLimiter;
use axum::{
    extract::{Request as AxRequest, State},
    middleware::{self, Next},
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
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                rate_limit_mw,
            ))
            .with_state(self.state.clone())
    }
}

/// Per-store rate-limit middleware (§21.8 429). Extracts the `{id}` segment
/// after `/stores/` and consumes a token; on exhaustion returns 429.
async fn rate_limit_mw(State(s): State<AppState>, req: AxRequest, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if let Some(rest) = path.strip_prefix("/stores/") {
        let id_seg = rest.split('/').next().unwrap_or("");
        if let Ok(store_id) = Bytes32::from_hex(id_seg) {
            if !s.rate_limiter.try_acquire(&store_id) {
                return RemoteError::RateLimited.into_response();
            }
        }
    }
    next.run(req).await
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
        // Never echo server-internal detail (filesystem paths, join/IO errors) in
        // a 5xx body — it leaks deployment information to any client. Log the
        // detail server-side and return a generic message. 4xx bodies describe the
        // request and are safe to surface.
        let body = match &self {
            RemoteError::Internal(detail) => {
                eprintln!("[digstore-remote] internal error: {detail}");
                "internal server error".to_string()
            }
            other => other.to_string(),
        };
        (self.status(), body).into_response()
    }
}
