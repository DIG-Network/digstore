//! Staging area management commands

pub mod diff;
pub mod list;

// Re-export the execute functions
pub use diff::execute as execute_diff;
pub use list::clear_staged;
pub use list::execute as execute_list;
