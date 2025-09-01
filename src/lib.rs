//! Digstore Min - A simplified content-addressable storage system
//!
//! Digstore Min provides Git-like repository functionality with enhanced merkle proof 
//! capabilities and URN-based retrieval. It focuses on core content-addressable storage
//! without encryption, privacy features, or blockchain integration.
//!
//! # Core Features
//!
//! - **Content-Addressable Storage**: Every piece of data identified by SHA-256 hash
//! - **Layer-Based Architecture**: Data organized into layers similar to Git commits
//! - **Merkle Proofs**: Generate cryptographic proofs for any data item or byte range
//! - **URN-Based Retrieval**: Permanent identifiers with byte range support
//! - **Portable Format**: Self-contained repositories that work anywhere
//!
//! # Example Usage
//!
//! ```rust,no_run
//! use digstore_min::{Store, StoreId};
//! use std::path::Path;
//!
//! // Initialize a new repository
//! let mut store = Store::init(Path::new("./my-project"))?;
//!
//! // Add files to the repository
//! store.add_files(&["src/main.rs", "Cargo.toml"])?;
//!
//! // Create a commit
//! let commit_id = store.commit("Initial commit")?;
//!
//! // Retrieve a file
//! let content = store.get_file(Path::new("src/main.rs"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod core;
pub mod storage;
pub mod proofs;
pub mod urn;
pub mod cli;
pub mod security;
pub mod ignore;
pub mod config;

// Re-export commonly used types
pub use core::{
    types::{Hash, StoreId, LayerType, Chunk, FileEntry},
    error::{DigstoreError, Result},
};

pub use storage::{
    store::Store,
    layer::Layer,
};

pub use urn::{Urn, ByteRange};

pub use proofs::{
    merkle::MerkleTree,
    proof::{Proof, ProofTarget},
};

/// Current version of Digstore Min
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Format version for compatibility
pub const FORMAT_VERSION: &str = "1.0";

/// Protocol version for URNs
pub const PROTOCOL_VERSION: &str = "1.0";
