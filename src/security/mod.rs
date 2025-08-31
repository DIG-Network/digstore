//! Security module for data scrambling and access control
//!
//! This module provides URN-based data scrambling and access control for Digstore Min.
//! All data stored in .layer files is scrambled using deterministic algorithms that
//! can only be reversed with the correct URN components.

pub mod scrambler;
pub mod access_control;
pub mod error;

// Re-export commonly used items
pub use scrambler::{DataScrambler, ScrambleState};
pub use access_control::{AccessController, AccessPermission, StoreAccessControl};
pub use error::{SecurityError, SecurityResult};
