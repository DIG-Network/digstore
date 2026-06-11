//! CLI error type and process exit-code mapping.

use digstore_core::ErrorCode;

/// Top-level CLI error. Every command returns `Result<_, CliError>`.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no digstore found at {0}; run `digstore init` first")]
    NoStore(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("non-fast-forward: remote root has advanced")]
    NonFastForward,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("no seed found; run `digstore seed import` or `digstore seed generate`")]
    NoSeed,
    #[error("wrong passphrase")]
    BadPassphrase,
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl CliError {
    /// Exit-code contract:
    /// 0 success | 1 other | 2 invalid-argument | 3 no-store | 4 not-found
    /// 5 verification-failed | 6 network | 7 non-fast-forward | 8 unauthorized.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::NoStore(_) => 3,
            CliError::InvalidArgument(_) => 2,
            CliError::NotFound(_) => 4,
            CliError::VerificationFailed(_) => 5,
            CliError::Network(_) => 6,
            CliError::NonFastForward => 7,
            CliError::Unauthorized(_) => 8,
            CliError::NoSeed => 9,
            CliError::BadPassphrase => 10,
            CliError::InvalidMnemonic(_) => 11,
            CliError::Other(_) => 1,
        }
    }

    /// A short, actionable fix suggestion for this error, if any.
    pub fn hint(&self) -> Option<String> {
        match self {
            CliError::NoStore(_) => Some("run `digstore init` to create a store here".into()),
            CliError::NonFastForward => Some("run `digstore pull` first, then push".into()),
            CliError::Unauthorized(_) => Some("check your credentials / store signing key".into()),
            CliError::NotFound(_) => Some("run `digstore log` to list generations and keys".into()),
            CliError::NoSeed => Some("run `digstore seed import` to set up your seed".into()),
            CliError::BadPassphrase => Some("re-run and enter the correct passphrase".into()),
            CliError::InvalidMnemonic(_) => Some("check the word list and word count (12/24)".into()),
            CliError::Network(_) => Some(
                "check your connection and that the remote is reachable; run `digstore remote list`".into(),
            ),
            CliError::VerificationFailed(_) => Some(
                "content failed verification — wrong salt/key or the store data was tampered with".into(),
            ),
            _ => None,
        }
    }

    /// Map a canonical `digstore-core` ErrorCode (from a host/guest call) to a CliError.
    pub fn from_error_code(code: ErrorCode, ctx: &str) -> Self {
        match code {
            ErrorCode::NotFound => CliError::NotFound(ctx.to_string()),
            ErrorCode::ValidationFailed => CliError::VerificationFailed(ctx.to_string()),
            ErrorCode::NetworkError | ErrorCode::Timeout => CliError::Network(ctx.to_string()),
            ErrorCode::NoSession | ErrorCode::SessionExpired => {
                CliError::Unauthorized(ctx.to_string())
            }
            _ => CliError::InvalidArgument(ctx.to_string()),
        }
    }
}

impl From<digstore_chain::ChainError> for CliError {
    fn from(e: digstore_chain::ChainError) -> Self {
        use digstore_chain::ChainError as C;
        match e {
            C::NoSeed(_) => CliError::NoSeed,
            C::Decrypt => CliError::BadPassphrase,
            C::InvalidMnemonic(m) => CliError::InvalidMnemonic(m),
            other => CliError::Other(anyhow::anyhow!(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct_and_nonzero() {
        let errs = [
            CliError::NoStore("x".into()),
            CliError::InvalidArgument("x".into()),
            CliError::NotFound("x".into()),
            CliError::VerificationFailed("x".into()),
            CliError::Network("x".into()),
            CliError::NonFastForward,
            CliError::Unauthorized("x".into()),
            CliError::NoSeed,
            CliError::BadPassphrase,
            CliError::InvalidMnemonic("x".into()),
        ];
        let mut codes: Vec<i32> = errs.iter().map(|e| e.exit_code()).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "exit codes must be distinct");
        assert!(codes.iter().all(|c| *c != 0), "exit codes must be nonzero");
    }

    #[test]
    fn maps_not_found_error_code() {
        let e = CliError::from_error_code(ErrorCode::NotFound, "urn:dig:...");
        assert!(matches!(e, CliError::NotFound(_)));
    }
}
