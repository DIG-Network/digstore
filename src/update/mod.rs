//! Automatic update system for Digstore CLI
//!
//! This module handles version checking and automatic updates

pub mod installer;
pub mod version_check;
pub mod version_manager;

pub use installer::{download_and_install_update, download_installer, InstallerType};
pub use version_check::{check_for_updates, UpdateInfo};
pub use version_manager::VersionManager;
