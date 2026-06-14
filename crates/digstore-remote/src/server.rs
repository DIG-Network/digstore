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

/// Maximum clock skew (seconds) between a request's signed timestamp and the
/// server's clock (paper §21.9). A signed request outside this window is rejected
/// — the bound (plus the random per-request nonce) is what defeats replay.
const AUTH_FRESHNESS_SECS: u64 = 300;

/// Shared server state behind an Arc; cloned into every handler.
#[derive(Clone)]
pub struct AppState {
    pub backend: Arc<dyn RemoteBackend>,
    pub rate_limiter: Arc<RateLimiter>,
    /// When true (the default for `new`/`with_rate_limiter`), EVERY request must
    /// carry valid §21.9 auth headers (a signed message from the caller's identity
    /// key) or it is rejected 401. `allow_anonymous()` turns this off for a
    /// fully-public read mirror or in-process tests.
    pub require_auth: bool,
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
                require_auth: true,
            },
        }
    }

    pub fn with_rate_limiter(backend: Arc<dyn RemoteBackend>, rl: Arc<RateLimiter>) -> Self {
        RemoteServer {
            state: AppState {
                backend,
                rate_limiter: rl,
                require_auth: true,
            },
        }
    }

    /// Disable per-request authentication: serve every route anonymously. For a
    /// fully-public read mirror, or for in-process handler tests that exercise the
    /// protocol logic rather than the auth layer. Builder-style.
    pub fn allow_anonymous(mut self) -> Self {
        self.state.require_auth = false;
        self
    }

    /// Build the axum Router exposing the full §21.2 surface.
    pub fn router(&self) -> Router {
        Router::new()
            .route(
                "/stores/:id",
                get(crate::handlers::descriptor::get_descriptor),
            )
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
            // dig RPC push protocol v1: init negotiation + presigned-finalize. A self-hosted node
            // has no object store to presign against, so init always negotiates INLINE and complete
            // is a protocol error (the body comes via PUT /module).
            .route(
                "/stores/:id/module/upload",
                post(crate::handlers::module::post_upload_init),
            )
            .route(
                "/stores/:id/module/complete",
                post(crate::handlers::module::post_complete),
            )
            .route(
                "/stores/:id/content",
                post(crate::handlers::content::post_content),
            )
            .route(
                "/stores/:id/proof",
                post(crate::handlers::proof::post_proof),
            )
            .route(
                "/stores/:id/delta",
                get(crate::handlers::delta::get_delta).post(crate::handlers::delta::post_delta),
            )
            .route(
                "/stores/:id/tombstone",
                post(crate::handlers::tombstone::post_tombstone),
            )
            // Order: auth runs FIRST (outermost), then rate-limit. An unauthenticated
            // request is rejected before it can consume a rate-limit token.
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                rate_limit_mw,
            ))
            .layer(middleware::from_fn_with_state(self.state.clone(), auth_mw))
            .with_state(self.state.clone())
    }

    /// Bind `addr` (host:port) and serve the §21 protocol until the process is
    /// terminated. Convenience for the runnable node (`digstore serve`) so callers
    /// need not wire up axum's serve machinery themselves.
    pub async fn serve(self, addr: &str) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, self.router().into_make_service()).await
    }
}

/// Per-request authentication middleware (paper §21.9). When `require_auth`, every
/// request must carry a fresh, valid signature from the caller's identity key over
/// the canonical request message; otherwise 401. No-op when auth is disabled
/// (`allow_anonymous`).
async fn auth_mw(State(s): State<AppState>, req: AxRequest, next: Next) -> Response {
    if !s.require_auth {
        return next.run(req).await;
    }
    match verify_request_auth(&req) {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

/// The logical operation a `(path, http method)` maps to — must be byte-identical
/// to the `method` the `DigClient` signs (so the verified message matches). Returns
/// None for a path the auth layer does not guard (it then passes through).
fn request_method_tag(path: &str, http_method: &http::Method) -> Option<&'static str> {
    let rest = path.strip_prefix("/stores/")?;
    // Drop the `{id}` segment; what remains identifies the route.
    let after_id = rest.split_once('/').map(|(_, r)| r).unwrap_or("");
    use http::Method;
    match (after_id, http_method) {
        ("", &Method::GET) => Some("fetch"),
        ("roots", &Method::GET) => Some("roots"),
        ("module", &Method::GET) | ("module", &Method::HEAD) => Some("module"),
        ("module", &Method::PUT) => Some("push"),
        // dig RPC push protocol v1 negotiation (init + presigned-finalize). Inline finalize reuses
        // the "push" tag on PUT /module above.
        ("module/upload", &Method::POST) => Some("push-init"),
        ("module/complete", &Method::POST) => Some("push-complete"),
        ("content", &Method::POST) => Some("content"),
        ("proof", &Method::POST) => Some("proof"),
        ("delta", &Method::GET) | ("delta", &Method::POST) => Some("delta"),
        ("tombstone", &Method::POST) => Some("tombstone"),
        _ => None,
    }
}

/// Verify the §21.9 auth headers on a request: a fresh, well-formed BLS signature
/// from `X-Dig-Identity` over `request_signing_message(method, store_id, ts, nonce)`.
/// This authenticates the CALLER (any valid identity is accepted for reads;
/// per-store push authorization is enforced separately by the module handler).
fn verify_request_auth(req: &AxRequest) -> Result<(), RemoteError> {
    let path = req.uri().path();
    let Some(method_tag) = request_method_tag(path, req.method()) else {
        // An unguarded path (should not happen for the routes we register) — allow.
        return Ok(());
    };
    let store_id = path
        .strip_prefix("/stores/")
        .and_then(|r| r.split('/').next())
        .ok_or_else(|| RemoteError::AuthFailed("no store id in path".into()))
        .and_then(parse_store_id)?;

    let h = req.headers();
    let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok());
    let identity_hex = get("x-dig-identity")
        .ok_or_else(|| RemoteError::AuthFailed("missing X-Dig-Identity".into()))?;
    let ts: u64 = get("x-dig-timestamp")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| RemoteError::AuthFailed("missing/invalid X-Dig-Timestamp".into()))?;
    let nonce_hex =
        get("x-dig-nonce").ok_or_else(|| RemoteError::AuthFailed("missing X-Dig-Nonce".into()))?;
    let auth_hex =
        get("x-dig-auth").ok_or_else(|| RemoteError::AuthFailed("missing X-Dig-Auth".into()))?;

    // Freshness window (defeats replay together with the per-request nonce).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.abs_diff(ts) > AUTH_FRESHNESS_SECS {
        return Err(RemoteError::AuthFailed("stale request timestamp".into()));
    }

    // Parse the identity public key, nonce, and signature.
    let pk_bytes = digstore_core::Bytes48::from_hex(identity_hex)
        .map_err(|_| RemoteError::AuthFailed("bad X-Dig-Identity hex".into()))?;
    let pk = digstore_crypto::bls::PublicKey::from_bytes(&pk_bytes)
        .map_err(|_| RemoteError::AuthFailed("bad identity public key".into()))?;
    let nonce: [u8; 32] = hex::decode(nonce_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| RemoteError::AuthFailed("bad X-Dig-Nonce".into()))?;
    let sig: [u8; 96] = hex::decode(auth_hex)
        .ok()
        .and_then(|b| <[u8; 96]>::try_from(b).ok())
        .ok_or_else(|| RemoteError::AuthFailed("bad X-Dig-Auth".into()))?;

    if !digstore_crypto::verify_request(
        &pk,
        method_tag,
        &store_id,
        ts,
        &nonce,
        &digstore_core::Bytes96(sig),
    ) {
        return Err(RemoteError::AuthFailed("signature does not verify".into()));
    }
    Ok(())
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
