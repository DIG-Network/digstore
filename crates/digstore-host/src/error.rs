//! Error type for the host runtime.

use digstore_core::abi::ErrorCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("wasmtime error: {0}")]
    Wasmtime(String),

    #[error("module validation failed: {0}")]
    Validation(String),

    #[error("guest export returned error code: {0:?}")]
    GuestError(ErrorCode),

    #[error("execution timed out")]
    Timeout,

    #[error("execution ran out of fuel")]
    OutOfFuel,

    #[error("guest exceeded memory ceiling")]
    MemoryLimit,

    #[error("missing required export: {0}")]
    MissingExport(&'static str),

    #[error("guest memory access out of bounds")]
    OutOfBounds,

    #[error("return buffer overflow: needed {needed}, max {max}")]
    ReturnBufferOverflow { needed: usize, max: usize },

    #[error("http error: {0}")]
    Http(String),
}

impl HostError {
    /// Map a negative guest sentinel (`is_error` true) to a `HostError`.
    /// Unknown / unmapped codes collapse to `GeneralError`.
    pub fn from_guest_code(code: i32) -> Self {
        let mapped = match code {
            c if c == ErrorCode::GeneralError as i32 => ErrorCode::GeneralError,
            c if c == ErrorCode::InvalidParameter as i32 => ErrorCode::InvalidParameter,
            c if c == ErrorCode::BufferTooSmall as i32 => ErrorCode::BufferTooSmall,
            c if c == ErrorCode::NoSession as i32 => ErrorCode::NoSession,
            c if c == ErrorCode::SessionExpired as i32 => ErrorCode::SessionExpired,
            c if c == ErrorCode::AttestationFailed as i32 => ErrorCode::AttestationFailed,
            c if c == ErrorCode::NetworkError as i32 => ErrorCode::NetworkError,
            c if c == ErrorCode::Timeout as i32 => ErrorCode::Timeout,
            c if c == ErrorCode::NotFound as i32 => ErrorCode::NotFound,
            c if c == ErrorCode::ValidationFailed as i32 => ErrorCode::ValidationFailed,
            _ => ErrorCode::GeneralError,
        };
        HostError::GuestError(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::abi::ErrorCode;

    #[test]
    fn from_error_code_maps_no_session() {
        let e = HostError::from_guest_code(ErrorCode::NoSession as i32);
        assert!(matches!(e, HostError::GuestError(ErrorCode::NoSession)));
    }

    #[test]
    fn from_error_code_maps_not_found() {
        let e = HostError::from_guest_code(ErrorCode::NotFound as i32);
        assert!(matches!(e, HostError::GuestError(ErrorCode::NotFound)));
    }

    #[test]
    fn unknown_code_is_general() {
        let e = HostError::from_guest_code(-9999);
        assert!(matches!(e, HostError::GuestError(ErrorCode::GeneralError)));
    }

    #[test]
    fn timeout_displays() {
        assert_eq!(HostError::Timeout.to_string(), "execution timed out");
    }
}
