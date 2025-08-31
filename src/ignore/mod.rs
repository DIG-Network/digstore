//! File ignore system with .digignore support
//!
//! This module provides comprehensive file filtering capabilities using .digignore files
//! that work exactly like .gitignore files. It supports all gitignore patterns including
//! wildcards, negation, directory patterns, and hierarchical ignore files.

pub mod parser;
pub mod checker;
pub mod scanner;

// Re-export commonly used items
pub use parser::{DigignoreParser, CompiledPattern, PatternType};
pub use checker::{IgnoreChecker, IgnoreResult};
pub use scanner::{FilteredFileScanner, ScanProgress, ScanPhase, ScanResult};
