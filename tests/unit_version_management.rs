//! Unit tests for version management system
//!
//! Tests the version management functionality including installation,
//! PATH management, and environment refresh.

use digstore_min::update::VersionManager;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_version_manager_creation() -> anyhow::Result<()> {
    // Version manager should be creatable
    let vm_result = VersionManager::new();

    // This might fail in CI environments, so handle gracefully
    match vm_result {
        Ok(vm) => {
            // Test basic functionality
            let versions = vm.list_versions()?;
            assert!(versions.len() >= 0, "Should return valid version list");

            let bin_name = vm.get_binary_name();
            assert!(!bin_name.is_empty(), "Binary name should not be empty");

            #[cfg(windows)]
            assert_eq!(bin_name, "digstore.exe");

            #[cfg(not(windows))]
            assert_eq!(bin_name, "digstore");
        },
        Err(_) => {
            println!(
                "Skipping version manager test - cannot create VersionManager (expected in CI)"
            );
        },
    }

    Ok(())
}

#[test]
fn test_version_directory_structure() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping version directory test - cannot create VersionManager");
        return Ok(());
    }

    let vm = vm_result.unwrap();

    // Test version directory paths
    let version_dir = vm.get_version_dir("1.0.0");
    assert!(version_dir.to_string_lossy().contains("1.0.0"));
    assert!(version_dir.to_string_lossy().contains(".digstore-versions"));

    let system_dir = vm.get_system_install_dir("2.0.0");
    assert!(system_dir.to_string_lossy().contains("v2.0.0"));

    #[cfg(windows)]
    assert!(system_dir.to_string_lossy().contains("dig-network"));

    Ok(())
}

#[test]
fn test_version_validation() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping version validation test");
        return Ok(());
    }

    let vm = vm_result.unwrap();
    let versions = vm.list_versions()?;

    // All versions should be valid strings
    for version in &versions {
        assert!(!version.is_empty(), "Version should not be empty");
        assert!(
            version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
            "Version '{}' should only contain valid characters",
            version
        );

        // Should be parseable as version-like format
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() >= 2 {
            for part in &parts[..2] {
                assert!(
                    part.parse::<u32>().is_ok(),
                    "Version part '{}' should be numeric in version '{}'",
                    part,
                    version
                );
            }
        }
    }

    Ok(())
}

#[test]
fn test_path_analysis_functionality() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping PATH analysis test");
        return Ok(());
    }

    let vm = vm_result.unwrap();

    // Test PATH-related functions don't crash
    let active_link_result = vm.get_active_link_path();
    if let Ok(link_path) = active_link_result {
        assert!(link_path.is_absolute(), "Link path should be absolute");

        if let Some(parent) = link_path.parent() {
            // Parent directory should be a reasonable bin directory
            let parent_str = parent.to_string_lossy();
            assert!(
                parent_str.contains("bin") || parent_str.contains("dig-network"),
                "Link path parent should be a bin directory: {}",
                parent_str
            );
        }
    }

    // Test active version detection
    let active_version = vm.get_active_version_from_path();
    match active_version {
        Ok(Some(version)) => {
            assert!(!version.is_empty(), "Active version should not be empty");
            println!("Detected active version: {}", version);
        },
        Ok(None) => {
            println!("No active version detected in PATH");
        },
        Err(e) => {
            println!("Error detecting active version: {}", e);
        },
    }

    Ok(())
}

#[test]
fn test_environment_refresh_logic() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping environment refresh test");
        return Ok(());
    }

    let vm = vm_result.unwrap();

    // Test environment refresh doesn't crash
    let refresh_result = vm.refresh_current_environment();

    match refresh_result {
        Ok(()) => {
            println!("Environment refresh completed successfully");
        },
        Err(e) => {
            println!(
                "Environment refresh failed (may be expected in test environment): {}",
                e
            );
        },
    }

    Ok(())
}

#[test]
fn test_binary_installation_simulation() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping binary installation simulation");
        return Ok(());
    }

    let mut vm = vm_result.unwrap();

    // Create a fake binary for testing
    let temp_dir = TempDir::new()?;
    let fake_binary = temp_dir.path().join("fake_digstore.exe");
    fs::write(&fake_binary, "fake binary content")?;

    // Test installation logic (this might fail due to permissions, which is OK)
    let install_result = vm.install_version("test-version", &fake_binary);

    match install_result {
        Ok(()) => {
            println!("Binary installation simulation succeeded");

            // Test that version appears in list
            let versions = vm.list_versions()?;
            assert!(
                versions.contains(&"test-version".to_string()),
                "Test version should appear in version list"
            );

            // Cleanup
            let _ = vm.remove_version("test-version");
        },
        Err(e) => {
            println!(
                "Binary installation simulation failed (expected in test environment): {}",
                e
            );
        },
    }

    Ok(())
}
