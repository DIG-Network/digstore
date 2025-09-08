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

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_strip)]
#![allow(clippy::type_complexity)]
#![allow(clippy::inherent_to_string)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::manual_clamp)]
#![allow(deprecated)]

pub mod cli;
pub mod config;
pub mod core;
pub mod crypto;
pub mod ignore;
pub mod proofs;
pub mod security;
pub mod storage;
pub mod urn;
pub mod wallet;

// Re-export commonly used types
pub use core::{
    error::{DigstoreError, Result},
    types::{Chunk, FileEntry, Hash, LayerType, StoreId},
};

pub use storage::{layer::Layer, store::Store};

pub use urn::{ByteRange, Urn};

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
