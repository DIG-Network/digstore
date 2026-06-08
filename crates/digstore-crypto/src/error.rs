use thiserror::Error;

/// Returned when AES-256-GCM authentication fails (ciphertext or tag tampered).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("AES-256-GCM authentication failed: ciphertext or tag was tampered")]
pub struct TamperError;

/// BLS-layer errors (malformed key/signature bytes).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlsError {
    #[error("invalid BLS public key bytes")]
    InvalidPublicKey,
    #[error("invalid BLS signature bytes")]
    InvalidSignature,
}

/// Umbrella crypto error returned by `decrypt_and_unwrap` and any caller that
/// wants a single error type spanning AEAD and BLS failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CryptoError {
    #[error(transparent)]
    Tamper(#[from] TamperError),
    #[error(transparent)]
    Bls(#[from] BlsError),
}
