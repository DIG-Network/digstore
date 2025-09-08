//! Automatic update system for Digstore CLI
//!
//! This module handles version checking and automatic updates

pub mod installer;
pub mod version_check;

pub use installer::{download_and_install_update, InstallerType};
pub use version_check::{check_for_updates, UpdateInfo};
