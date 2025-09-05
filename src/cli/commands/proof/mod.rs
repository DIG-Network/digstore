//! Proof system commands

pub mod generate;
pub mod generate_archive_size;
pub mod verify;
pub mod verify_archive_size;

// Re-export the execute functions
pub use generate::execute as execute_generate;
pub use generate_archive_size::execute as execute_generate_archive_size;
pub use verify::execute as execute_verify;
pub use verify_archive_size::execute as execute_verify_archive_size;
