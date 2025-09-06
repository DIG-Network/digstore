//! CLI command implementations

// Core commands (top-level)
pub mod add;
pub mod cat;
pub mod coin;
pub mod commit;
pub mod completion;
pub mod config;
pub mod decrypt;
pub mod get;
pub mod init;
pub mod keygen;
pub mod status;
pub mod wallet;

// Command subtrees
pub mod layer;
pub mod proof;
pub mod staged;
pub mod store;

// Common utilities for commands
use crate::core::error::DigstoreError;
use anyhow::Result;
use std::path::Path;

/// Check if we're in a repository directory (has .digstore file)
pub fn find_repository_root() -> Result<Option<std::path::PathBuf>> {
    let mut current_dir = std::env::current_dir()?;

    loop {
        let digstore_file = current_dir.join(".digstore");
        if digstore_file.exists() {
            return Ok(Some(current_dir));
        }

        if let Some(parent) = current_dir.parent() {
            current_dir = parent.to_path_buf();
        } else {
            break;
        }
    }

    Ok(None)
}

/// Load store ID from .digstore file
pub fn load_store_id_from_digstore(repo_root: &Path) -> Result<crate::core::types::StoreId> {
    let digstore_file = repo_root.join(".digstore");
    let content = std::fs::read_to_string(digstore_file)?;

    // Parse TOML content
    let parsed: toml::Value = content.parse()?;

    if let Some(store_id_str) = parsed.get("store_id").and_then(|v| v.as_str()) {
        crate::core::types::Hash::from_hex(store_id_str).map_err(|e| {
            DigstoreError::invalid_store_id(format!("Invalid store ID in .digstore: {}", e)).into()
        })
    } else {
        Err(DigstoreError::invalid_store_id("No store_id found in .digstore file").into())
    }
}
