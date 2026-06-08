//! Error codes shared across the WASM ABI and core failures.

use alloc::string::String;

/// ABI error codes returned across the host/guest boundary (negative i32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    GeneralError = -1,
    InvalidParameter = -2,
    BufferTooSmall = -3,
    NoSession = -100,
    SessionExpired = -101,
    AttestationFailed = -102,
    NetworkError = -200,
    Timeout = -203,
    NotFound = -300,
    ValidationFailed = -301,
}

impl ErrorCode {
    /// Recover an `ErrorCode` from its i32 discriminant, if it is a known code.
    pub const fn from_i32(value: i32) -> Option<ErrorCode> {
        match value {
            -1 => Some(ErrorCode::GeneralError),
            -2 => Some(ErrorCode::InvalidParameter),
            -3 => Some(ErrorCode::BufferTooSmall),
            -100 => Some(ErrorCode::NoSession),
            -101 => Some(ErrorCode::SessionExpired),
            -102 => Some(ErrorCode::AttestationFailed),
            -200 => Some(ErrorCode::NetworkError),
            -203 => Some(ErrorCode::Timeout),
            -300 => Some(ErrorCode::NotFound),
            -301 => Some(ErrorCode::ValidationFailed),
            _ => None,
        }
    }
}

/// Library-level error for parsing/codec/validation failures inside the core crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// A string could not be parsed (URN, hex, etc.).
    Parse(String),
    /// A codec decode failed.
    Decode(String),
    /// A value failed validation.
    Validation(String),
}
