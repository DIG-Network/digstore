//! Regression tests for user configuration fixes
//!
//! These tests ensure that email remains optional in user configuration
//! and that configuration serialization works properly.

use digstore_min::config::GlobalConfig;
use std::fs;
use tempfile::TempDir;

/// Test for user configuration with optional email regression
/// This test ensures that email is properly optional in user configuration
#[test]
fn test_user_config_optional_email() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a test config in the temp directory
    let config_dir = temp_dir.path().join(".dig");
    fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.toml");

    // Test 1: Configuration with name only (no email) should be valid
    let config_content = r#"
[user]
name = "Test User"

[core]

[crypto]

[wallet]
"#;
    fs::write(&config_path, config_content)?;

    // This should not fail - email should be optional
    let config_result = toml::from_str::<GlobalConfig>(config_content);
    assert!(
        config_result.is_ok(),
        "Config with name only should be valid: {:?}",
        config_result.err()
    );

    let config = config_result.unwrap();
    assert!(config.user.name.is_some(), "Name should be present");
    assert!(
        config.user.email.is_none() || config.user.email.as_ref().map_or(false, |e| e.is_empty()),
        "Email should be None or empty string when not provided"
    );

    // Test 2: Configuration with both name and email should be valid
    let config_content_with_email = r#"
[user]
name = "Test User"
email = "test@example.com"

[core]

[crypto]

[wallet]
"#;
    let config_result = toml::from_str::<GlobalConfig>(config_content_with_email);
    assert!(
        config_result.is_ok(),
        "Config with name and email should be valid: {:?}",
        config_result.err()
    );

    let config = config_result.unwrap();
    assert!(config.user.name.is_some(), "Name should be present");
    assert!(config.user.email.is_some(), "Email should be present when provided");

    // Test 3: Configuration with empty email should be valid
    let config_content_empty_email = r#"
[user]
name = "Test User"
email = ""

[core]

[crypto]

[wallet]
"#;
    let config_result = toml::from_str::<GlobalConfig>(config_content_empty_email);
    assert!(
        config_result.is_ok(),
        "Config with empty email should be valid: {:?}",
        config_result.err()
    );

    let config = config_result.unwrap();
    assert!(config.user.name.is_some(), "Name should be present");
    assert!(
        config.user.email.is_some() && config.user.email.as_ref().unwrap().is_empty(),
        "Email should be empty string when explicitly set to empty"
    );

    Ok(())
}

/// Test that user configuration validation logic works correctly
#[test]
fn test_user_config_validation() -> anyhow::Result<()> {
    // Test is_user_configured logic
    let mut config = GlobalConfig::default();
    
    // Should not be configured initially
    assert!(!config.is_user_configured(), "Should not be configured without name");
    
    // Should be configured with just name (email optional)
    config.user.name = Some("Test User".to_string());
    assert!(config.is_user_configured(), "Should be configured with name only");
    
    // Should still be configured with empty email
    config.user.email = Some("".to_string());
    assert!(config.is_user_configured(), "Should be configured with empty email");
    
    // Should be configured with both name and email
    config.user.email = Some("test@example.com".to_string());
    assert!(config.is_user_configured(), "Should be configured with name and email");

    Ok(())
}

/// Test configuration serialization and deserialization
#[test]
fn test_config_serialization_roundtrip() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let config_path = temp_dir.path().join("test_config.toml");

    // Create config with optional email scenarios
    let test_configs = vec![
        // Name only
        GlobalConfig {
            user: digstore_min::config::UserConfig {
                name: Some("Test User".to_string()),
                email: None,
            },
            ..Default::default()
        },
        // Name and email
        GlobalConfig {
            user: digstore_min::config::UserConfig {
                name: Some("Test User".to_string()),
                email: Some("test@example.com".to_string()),
            },
            ..Default::default()
        },
        // Name and empty email
        GlobalConfig {
            user: digstore_min::config::UserConfig {
                name: Some("Test User".to_string()),
                email: Some("".to_string()),
            },
            ..Default::default()
        },
    ];

    for (i, config) in test_configs.iter().enumerate() {
        // Serialize to TOML
        let toml_content = toml::to_string_pretty(config)?;
        fs::write(&config_path, &toml_content)?;

        // Deserialize back
        let loaded_config: GlobalConfig = toml::from_str(&toml_content)?;

        // Verify round-trip accuracy
        assert_eq!(
            loaded_config.user.name, config.user.name,
            "Config {} name should round-trip correctly", i
        );
        assert_eq!(
            loaded_config.user.email, config.user.email,
            "Config {} email should round-trip correctly", i
        );

        // Verify validation works
        assert!(
            loaded_config.is_user_configured(),
            "Config {} should be valid after round-trip", i
        );
    }

    Ok(())
}
