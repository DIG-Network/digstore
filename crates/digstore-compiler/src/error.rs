use thiserror::Error;

/// Errors produced by the dig-compiler pipeline.
#[derive(Debug, Error)]
pub enum CompilerError {
    /// At least one trusted host key is required (paper §5.3, §19.2).
    #[error("compilation requires at least one trusted host key")]
    NoTrustedKeys,
    /// The prebuilt guest template was malformed or missing a required export
    /// or violated memory bounds (§5.1).
    #[error("invalid guest template: {0}")]
    InvalidTemplate(String),
    /// A generation directory could not be loaded.
    #[error("generation load failed: {0}")]
    GenerationLoad(String),
    /// The WASM module failed re-validation after data injection / obfuscation.
    #[error("emitted module failed validation: {0}")]
    Validation(String),
    /// A key-table entry referenced a chunk index outside the chunk index.
    #[error("key table references missing chunk index {0}")]
    MissingChunk(u32),
    /// I/O failure during atomic write or template load.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, CompilerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trusted_keys_renders_documented_message() {
        let e = CompilerError::NoTrustedKeys;
        assert_eq!(
            e.to_string(),
            "compilation requires at least one trusted host key"
        );
    }

    #[test]
    fn invalid_template_carries_reason() {
        let e = CompilerError::InvalidTemplate("missing export get_content".into());
        assert!(e.to_string().contains("missing export get_content"));
    }
}
