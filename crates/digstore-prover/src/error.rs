use thiserror::Error;

/// Errors produced by provers, verifiers, and chain sources (§13).
#[derive(Debug, Error)]
pub enum ProverError {
    #[error("program hash mismatch: expected {expected}, got {actual}")]
    ProgramHashMismatch { expected: String, actual: String },
    #[error("public output mismatch")]
    PublicOutputMismatch,
    #[error("public input commitment mismatch")]
    PublicInputMismatch,
    #[error("nonce binding mismatch: proof bound to a different request")]
    NonceMismatch,
    #[error("response roothash {0} is not in the trusted set")]
    UntrustedRoot(String),
    #[error("proof is bound to roothash {bound}, but response asserts {asserted}")]
    RootBindingMismatch { bound: String, asserted: String },
    #[error("node signature verification failed")]
    NodeSignatureInvalid,
    #[error("chain block outside freshness window: block ts {block_ts}, now {now}, window {window}s")]
    BlockTooOld { block_ts: u64, now: u64, window: u64 },
    #[error("chain block timestamp {0} is in the future relative to now {1}")]
    BlockInFuture(u64, u64),
    #[error("block not found on trusted chain: {0}")]
    BlockNotOnChain(String),
    #[error("zk proof verification failed: {0}")]
    ZkProofInvalid(String),
    #[error("hardware attestation verification failed: {0}")]
    AttestationInvalid(String),
    #[error("proving backend failure: {0}")]
    Backend(String),
    #[error("codec error: {0}")]
    Codec(String),
    #[error("chain RPC error: {0}")]
    ChainRpc(String),
}

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, ProverError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_descriptive() {
        let e = ProverError::ProgramHashMismatch { expected: "aa".into(), actual: "bb".into() };
        assert_eq!(e.to_string(), "program hash mismatch: expected aa, got bb");
    }

    #[test]
    fn root_binding_error_display() {
        let e = ProverError::RootBindingMismatch { bound: "aa".into(), asserted: "bb".into() };
        assert_eq!(e.to_string(), "proof is bound to roothash aa, but response asserts bb");
    }
}
