use std::path::PathBuf;

/// Result alias used throughout digstore-store.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Errors produced by store operations (init/open/add/commit/diff).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store already exists at {0}")]
    AlreadyExists(String),

    #[error("store not found at {0}")]
    NotFound(String),

    #[error("invalid store configuration: {0}")]
    InvalidConfig(String),

    #[error("staging area corrupt: {0}")]
    CorruptStaging(String),

    #[error("generation {0} not found")]
    GenerationNotFound(String),

    #[error("chunk {0} not found in any generation")]
    ChunkNotFound(String),

    #[error("root history is not monotonic: generation id {got} follows {last}")]
    NonMonotonicHistory { last: u64, got: u64 },

    #[error("nothing staged to commit")]
    EmptyStaging,

    #[error("manifest parse error: {0}")]
    Manifest(String),

    #[error("config (de)serialization error: {0}")]
    Config(String),

    #[error("path is not under the staging base: {0}")]
    PathEscape(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_already_exists_displays_path() {
        let e = StoreError::AlreadyExists("/tmp/x".into());
        assert_eq!(e.to_string(), "store already exists at /tmp/x");
    }

    #[test]
    fn io_error_wraps_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let e: StoreError = io.into();
        assert!(matches!(e, StoreError::Io(_)));
    }
}
