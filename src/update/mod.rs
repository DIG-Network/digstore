//! Automatic update system for Digstore CLI
//!
//! This module handles version checking and automatic updates

pub mod version_check;
pub mod installer;

pub use version_check::{check_for_updates, UpdateInfo};
pub use installer::{download_and_install_update, InstallerType};
