use http::StatusCode;
use thiserror::Error;

/// Server-side error. Each variant maps to a §21.8 status code.
#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("unknown store")]
    UnknownStore,
    #[error("unknown root")]
    UnknownRoot,
    #[error("push not authorized: {0}")]
    Unauthorized(String),
    #[error("request authentication failed: {0}")]
    AuthFailed(String),
    #[error("missing bearer token")]
    MissingBearer,
    #[error("non-fast-forward push")]
    NonFastForward,
    #[error("module too large: {0} bytes")]
    TooLarge(u64),
    #[error("module failed validation: {0}")]
    Validation(String),
    #[error("rate limited")]
    RateLimited,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl RemoteError {
    /// §21.8 status code mapping. Note: content miss is NEVER mapped here
    /// (it returns 200 with a decoy); only structural errors map.
    pub fn status(&self) -> StatusCode {
        match self {
            RemoteError::UnknownStore | RemoteError::UnknownRoot => StatusCode::NOT_FOUND,
            RemoteError::Unauthorized(_) => StatusCode::FORBIDDEN,
            RemoteError::AuthFailed(_) => StatusCode::UNAUTHORIZED,
            RemoteError::MissingBearer => StatusCode::UNAUTHORIZED,
            RemoteError::NonFastForward => StatusCode::CONFLICT,
            RemoteError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            RemoteError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            RemoteError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            RemoteError::BadRequest(_) => StatusCode::BAD_REQUEST,
            RemoteError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Client-side error for DigClient operations.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http transport: {0}")]
    Transport(String),
    #[error("server returned status {0}")]
    Status(u16),
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("non-fast-forward (409)")]
    NonFastForward,
    /// Server returned 4xx/5xx with a JSON `{"error":…,"message":"…"}` body.
    /// `message` is the human-readable server reason (or the raw body if not JSON).
    #[error("remote rejected ({status}): {message}")]
    Remote { status: u16, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_matches_spec_21_8() {
        assert_eq!(RemoteError::UnknownStore.status(), StatusCode::NOT_FOUND);
        assert_eq!(RemoteError::UnknownRoot.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            RemoteError::Unauthorized("bad sig".into()).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            RemoteError::MissingBearer.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(RemoteError::NonFastForward.status(), StatusCode::CONFLICT);
        assert_eq!(
            RemoteError::TooLarge(1).status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            RemoteError::Validation("x".into()).status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            RemoteError::RateLimited.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
