//! Proof system commands

pub mod generate;
pub mod verify;

// Re-export the execute functions
pub use generate::execute as execute_generate;
pub use verify::execute as execute_verify;
