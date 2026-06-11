//! Bridge for calling async anchoring ops from the synchronous command
//! dispatch. One current-thread tokio runtime per call site.

use crate::error::CliError;
use std::future::Future;

/// Runs an async future to completion on a fresh current-thread runtime.
/// Used by `init`/`commit`/`anchor` to drive `ChainAnchor` (async) from the
/// sync command handlers.
pub fn block_on<F: Future>(fut: F) -> Result<F::Output, CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(anyhow::anyhow!("tokio runtime: {e}")))?;
    Ok(rt.block_on(fut))
}
