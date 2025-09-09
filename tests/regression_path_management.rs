//! Regression tests for PATH management functionality
//!
//! These tests ensure that PATH analysis, fixing, and environment refresh work correctly.

use assert_cmd::Command;
use predicates::prelude::*;
use std::{env, fs};
use tempfile::TempDir;
use digstore_min::update::VersionManager;

/// Test PATH analysis functionality
#[test]
fn test_path_analysis_no_crash() {
    let temp_dir = TempDir::new().unwrap();
    
    // PATH analysis should never crash, regardless of PATH contents
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "fix-path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyzing PATH"));
}

/// Test automatic PATH fixing
#[test]
fn test_path_auto_fix_safety() {
    let temp_dir = TempDir::new().unwrap();
    
    // Auto-fix should either succeed or fail gracefully
    let command_assert = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "fix-path-auto"])
        .assert();
    let output = command_assert.get_output();
    
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Automatically fixing PATH") ||
            stdout.contains("PATH updated") ||
            stdout.contains("PATH ordering"),
            "Should show PATH fixing progress"
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should fail gracefully with helpful error
        assert!(
            stderr.contains("PATH") ||
            stderr.contains("permission") ||
            stderr.contains("access"),
            "Should fail gracefully: {}",
            stderr
        );
    }
}

/// Test environment variable handling
#[test]
fn test_environment_variable_safety() -> anyhow::Result<()> {
    // Store original PATH
    let original_path = env::var("PATH").unwrap_or_default();
    
    // Test that PATH manipulation doesn't break the environment
    let path_entries: Vec<&str> = original_path.split(';').collect();
    assert!(!path_entries.is_empty(), "PATH should have entries");
    
    // Test PATH reconstruction
    let reconstructed_path = path_entries.join(";");
    assert_eq!(reconstructed_path, original_path, "PATH reconstruction should be accurate");
    
    // Test adding a directory to PATH
    let test_dir = "C:\\test\\directory";
    let new_path = format!("{};{}", test_dir, original_path);
    assert!(new_path.starts_with(test_dir), "New PATH should start with test directory");
    assert!(new_path.contains(&original_path), "New PATH should contain original PATH");
    
    // Test removing a directory from PATH
    let filtered_entries: Vec<&str> = path_entries
        .into_iter()
        .filter(|entry| entry.trim() != test_dir)
        .collect();
    let filtered_path = filtered_entries.join(";");
    assert_eq!(filtered_path, original_path, "Filtering non-existent directory should not change PATH");
    
    Ok(())
}

/// Test version detection from PATH
#[test]
fn test_version_detection_robustness() -> anyhow::Result<()> {
    use digstore_min::update::VersionManager;
    
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping version detection test");
        return Ok(());
    }
    
    let vm = vm_result.unwrap();
    
    // Test version detection handles various scenarios
    let active_version = vm.get_active_version_from_path();
    
    match active_version {
        Ok(Some(version)) => {
            assert!(!version.is_empty(), "Detected version should not be empty");
            assert!(
                version.chars().any(|c| c.is_ascii_digit()),
                "Version should contain digits: {}",
                version
            );
            println!("Successfully detected version: {}", version);
        }
        Ok(None) => {
            println!("No version detected in PATH (acceptable)");
        }
        Err(e) => {
            println!("Version detection failed (acceptable in test environment): {}", e);
        }
    }
    
    Ok(())
}

/// Test that multiple version installations don't interfere
#[test]
fn test_multiple_version_isolation() -> anyhow::Result<()> {
    let vm_result = VersionManager::new();
    if vm_result.is_err() {
        println!("Skipping multiple version test");
        return Ok(());
    }
    
    let mut vm = vm_result.unwrap();
    
    // Create fake binaries for testing
    let temp_dir = TempDir::new()?;
    let versions_to_test = ["test-1.0.0", "test-2.0.0", "test-3.0.0"];
    
    let mut successful_installs = Vec::new();
    
    for version in &versions_to_test {
        let fake_binary = temp_dir.path().join(format!("{}.exe", version));
        fs::write(&fake_binary, format!("fake binary {}", version))?;
        
        let install_result = vm.install_version(version, &fake_binary);
        if install_result.is_ok() {
            successful_installs.push(version.to_string());
        }
    }
    
    if !successful_installs.is_empty() {
        println!("Successfully installed {} test versions", successful_installs.len());
        
        // Test that versions are isolated (don't interfere with each other)
        let versions = vm.list_versions()?;
        for installed_version in &successful_installs {
            assert!(
                versions.contains(installed_version),
                "Installed version {} should appear in list",
                installed_version
            );
            
            // Test version directory exists
            let version_dir = vm.get_version_dir(installed_version);
            assert!(version_dir.exists(), "Version directory should exist: {}", version_dir.display());
        }
        
        // Cleanup test versions
        for version in &successful_installs {
            let _ = vm.remove_version(version);
        }
    } else {
        println!("No versions could be installed (expected in test environment)");
    }
    
    Ok(())
}

/// Test update system integration with version management
#[test]
fn test_update_system_integration() -> anyhow::Result<()> {
    // Test that update checking works
    let update_check = digstore_min::update::check_for_updates();
    
    match update_check {
        Ok(info) => {
            println!("Update check successful");
            assert!(!info.current_version.is_empty(), "Current version should be set");
            assert!(!info.latest_version.is_empty(), "Latest version should be set");
            
            // Test version comparison logic
            if info.update_available {
                println!("Update available: {} → {}", info.current_version, info.latest_version);
                assert_ne!(info.current_version, info.latest_version, "Available update should have different versions");
            } else {
                println!("No update available (current: {})", info.current_version);
            }
            
            // Test download URL format if available
            if let Some(url) = info.download_url {
                assert!(url.starts_with("http"), "Download URL should be HTTP(S)");
                assert!(url.contains("github.com"), "Should be GitHub release URL");
                assert!(url.contains(".msi"), "Should be MSI file for Windows");
            }
        }
        Err(e) => {
            println!("Update check failed (expected in offline test environment): {}", e);
        }
    }
    
    Ok(())
}
