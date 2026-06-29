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
    #[error("insufficient {asset}: need {need}, have {have}; fund {address}")]
    InsufficientFunds {
        need: u64,
        have: u64,
        address: String,
        asset: String,
    },
    #[error("chain error: {0}")]
    Chain(String),
    #[error("onchain confirmation timed out")]
    ConfirmTimeout,
    #[error("mint failed: {0}")]
    MintFailed(String),
    #[error("update failed: {0}")]
    UpdateFailed(String),
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
            CliError::InsufficientFunds { .. } => 12,
            CliError::Chain(_) => 13,
            CliError::ConfirmTimeout => 14,
            CliError::MintFailed(_) => 15,
            CliError::UpdateFailed(_) => 16,
            CliError::Other(_) => 1,
        }
    }

    /// Stable, machine-readable error code (UPPER_SNAKE), drawn from the variant —
    /// NEVER derived from the human message. Agents branch on this (and/or
    /// [`Self::exit_code`]) instead of string-matching prose. The code ↔ exit-code
    /// mapping is 1:1 and is also emitted by `--help-json` and documented in the
    /// README exit-code table.
    pub fn code(&self) -> &'static str {
        match self {
            CliError::NoStore(_) => "NO_STORE",
            CliError::InvalidArgument(_) => "INVALID_ARGUMENT",
            CliError::NotFound(_) => "NOT_FOUND",
            CliError::VerificationFailed(_) => "VERIFICATION_FAILED",
            CliError::Network(_) => "NETWORK",
            CliError::NonFastForward => "NON_FAST_FORWARD",
            CliError::Unauthorized(_) => "UNAUTHORIZED",
            CliError::NoSeed => "NO_SEED",
            CliError::BadPassphrase => "BAD_PASSPHRASE",
            CliError::InvalidMnemonic(_) => "INVALID_MNEMONIC",
            CliError::InsufficientFunds { .. } => "INSUFFICIENT_FUNDS",
            CliError::Chain(_) => "CHAIN",
            CliError::ConfirmTimeout => "CONFIRM_TIMEOUT",
            CliError::MintFailed(_) => "MINT_FAILED",
            CliError::UpdateFailed(_) => "UPDATE_FAILED",
            CliError::Other(_) => "ERROR",
        }
    }

    /// The full, static exit-code table as `(code, exit_code, meaning)` rows — the
    /// single source of truth for both `--help-json`'s `exit_codes` and the README
    /// table, so they cannot drift from [`Self::code`]/[`Self::exit_code`]. Row 0 is
    /// the success row (`OK`, exit 0); the rest mirror the error variants.
    pub fn exit_code_table() -> &'static [(&'static str, i32, &'static str)] {
        &[
            ("OK", 0, "success"),
            ("ERROR", 1, "an unclassified error"),
            (
                "INVALID_ARGUMENT",
                2,
                "a bad/missing argument or flag value",
            ),
            ("NO_STORE", 3, "no digstore workspace/store found here"),
            (
                "NOT_FOUND",
                4,
                "the requested resource/root/key was not found",
            ),
            (
                "VERIFICATION_FAILED",
                5,
                "content failed verification (tamper, wrong salt/key)",
            ),
            ("NETWORK", 6, "a network/remote error"),
            (
                "NON_FAST_FORWARD",
                7,
                "the remote root advanced; pull before pushing",
            ),
            (
                "UNAUTHORIZED",
                8,
                "missing/invalid credentials or signing key",
            ),
            ("NO_SEED", 9, "no wallet seed is set up"),
            ("BAD_PASSPHRASE", 10, "wrong seed passphrase"),
            ("INVALID_MNEMONIC", 11, "the BIP-39 mnemonic is invalid"),
            (
                "INSUFFICIENT_FUNDS",
                12,
                "not enough XCH or DIG to complete the spend",
            ),
            ("CHAIN", 13, "a Chia chain / coinset.org error"),
            (
                "CONFIRM_TIMEOUT",
                14,
                "on-chain confirmation timed out (resumable)",
            ),
            ("MINT_FAILED", 15, "the on-chain mint failed"),
            ("UPDATE_FAILED", 16, "the on-chain root update failed"),
        ]
    }

    /// A short, actionable fix suggestion for this error, if any.
    pub fn hint(&self) -> Option<String> {
        match self {
            CliError::NoStore(_) => Some("run `digstore init` to create a store here".into()),
            CliError::NonFastForward => Some("run `digstore pull` first, then push".into()),
            CliError::Unauthorized(_) => Some("check your credentials / store signing key".into()),
            CliError::NotFound(_) => Some("run `digstore log` to list capsules and keys".into()),
            CliError::NoSeed => Some("run `digstore seed import` to set up your seed".into()),
            CliError::BadPassphrase => Some("re-run and enter the correct passphrase".into()),
            CliError::InvalidMnemonic(_) => Some("check the word list and word count (12/24)".into()),
            CliError::Network(_) => Some(
                "check your connection and that the remote is reachable; run `digstore remote list`".into(),
            ),
            CliError::VerificationFailed(_) => Some(
                "content failed verification — wrong salt/key or the store data was tampered with".into(),
            ),
            // For a DIG shortfall, point the user at where to acquire $DIG (the
            // three canonical venues) — funding is off-CLI, so a dead-end here
            // would otherwise leave them stuck. XCH keeps the bare receive-address
            // line (any Chia exchange / wallet sends XCH).
            CliError::InsufficientFunds { address, asset, .. } if asset == "DIG" => Some(format!(
                "send DIG to {address}, then retry. {}",
                crate::branding::get_dig_hint()
            )),
            CliError::InsufficientFunds { address, asset, .. } => Some(format!("send {asset} to {address}, then retry")),
            CliError::Chain(_) => Some("check your connection to coinset.org and retry".into()),
            CliError::ConfirmTimeout => Some("the transaction may still confirm; run `digstore anchor status`".into()),
            CliError::MintFailed(_) | CliError::UpdateFailed(_) => Some("retry; if it persists, check wallet funds and coinset.org".into()),
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
            C::Chain(m) => CliError::Chain(m),
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
            CliError::InsufficientFunds {
                need: 1,
                have: 0,
                address: "xch1test".into(),
                asset: "XCH".into(),
            },
            CliError::Chain("x".into()),
            CliError::ConfirmTimeout,
            CliError::MintFailed("x".into()),
            CliError::UpdateFailed("x".into()),
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

    #[test]
    fn codes_are_distinct_upper_snake_and_match_exit_codes() {
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
            CliError::InsufficientFunds {
                need: 1,
                have: 0,
                address: "xch1test".into(),
                asset: "XCH".into(),
            },
            CliError::Chain("x".into()),
            CliError::ConfirmTimeout,
            CliError::MintFailed("x".into()),
            CliError::UpdateFailed("x".into()),
        ];
        let mut codes: Vec<&str> = errs.iter().map(|e| e.code()).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "codes must be distinct");
        for c in &codes {
            assert!(
                c.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_'),
                "code `{c}` must be UPPER_SNAKE"
            );
        }
        // Every variant's (code, exit_code) appears in the table; the table is 1:1.
        for e in &errs {
            let row = CliError::exit_code_table()
                .iter()
                .find(|(code, _, _)| *code == e.code())
                .unwrap_or_else(|| panic!("code {} missing from table", e.code()));
            assert_eq!(row.1, e.exit_code(), "exit code mismatch for {}", e.code());
        }
    }

    #[test]
    fn exit_code_table_has_success_row_and_unique_codes() {
        let table = CliError::exit_code_table();
        assert_eq!(table[0], ("OK", 0, "success"));
        let mut exits: Vec<i32> = table.iter().map(|(_, x, _)| *x).collect();
        let n = exits.len();
        exits.sort_unstable();
        exits.dedup();
        assert_eq!(exits.len(), n, "exit codes in the table must be unique");
    }
}
