//! Security module for data scrambling and access control
//!
//! This module provides URN-based data scrambling and access control for Digstore Min.
//! All data stored in .dig files is scrambled using deterministic algorithms that
//! can only be reversed with the correct URN components.

pub mod access_control;
pub mod error;
pub mod scrambler;

// Re-export commonly used items
pub use error::{SecurityError, SecurityResult};
pub use scrambler::DataScrambler;
