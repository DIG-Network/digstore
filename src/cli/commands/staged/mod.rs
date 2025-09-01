//! Staging area management commands

pub mod list;
pub mod diff;

// Re-export the execute functions
pub use list::execute as execute_list;
pub use list::clear_staged;
pub use diff::execute as execute_diff;
