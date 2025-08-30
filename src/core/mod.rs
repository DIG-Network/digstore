//! Core types and utilities for Digstore Min
//!
//! This module contains the fundamental data types, error handling,
//! and utility functions used throughout the system.

pub mod types;
pub mod error;
pub mod hash;
pub mod digstore_file;

// Re-export commonly used items
pub use types::{Hash, StoreId, LayerType, Chunk, FileEntry, CommitInfo};
pub use error::{DigstoreError, Result};
pub use hash::{sha256, hash_bytes, hash_file};
pub use digstore_file::DigstoreFile;
