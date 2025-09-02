//! Core types and utilities for Digstore Min
//!
//! This module contains the fundamental data types, error handling,
//! and utility functions used throughout the system.

pub mod digstore_file;
pub mod error;
pub mod hash;
pub mod types;

// Re-export commonly used items
pub use digstore_file::DigstoreFile;
pub use error::{DigstoreError, Result};
pub use hash::{hash_bytes, hash_file, sha256};
pub use types::{Chunk, CommitInfo, FileEntry, Hash, LayerType, StoreId};
