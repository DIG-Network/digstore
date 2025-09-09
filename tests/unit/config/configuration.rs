//! Unit tests for configuration management
//!
//! Tests global and store-specific configuration functionality.

use digstore_min::config::GlobalConfig;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_global_config_creation() {
    let config = GlobalConfig::default();
    
    // Default config should have empty user settings
    assert!(config.user.name.is_none());
    assert!(config.user.email.is_none());
    
    // Should not be considered configured without name
    assert!(!config.is_user_configured());
}

#[test]
fn test_config_serialization() -> anyhow::Result<()> {
    let mut config = GlobalConfig::default();
    config.user.name = Some("Test User".to_string());
    config.user.email = Some("test@example.com".to_string());
    
    // Serialize to TOML
    let toml_content = toml::to_string_pretty(&config)?;
    
    // Should contain user information
    assert!(toml_content.contains("Test User"));
    assert!(toml_content.contains("test@example.com"));
    
    // Deserialize back
    let loaded_config: GlobalConfig = toml::from_str(&toml_content)?;
    
    assert_eq!(loaded_config.user.name, config.user.name);
    assert_eq!(loaded_config.user.email, config.user.email);
    
    Ok(())
}

#[test]
fn test_config_file_operations() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create temporary config directory
    let config_dir = temp_dir.path().join(".dig");
    fs::create_dir_all(&config_dir)?;
    
    let mut config = GlobalConfig::default();
    config.user.name = Some("File Test User".to_string());
    
    // Test config path generation
    let config_path = GlobalConfig::get_config_path();
    assert!(config_path.is_ok());
    
    Ok(())
}

#[test]
fn test_config_validation_rules() {
    let mut config = GlobalConfig::default();
    
    // Empty config should not be valid
    assert!(!config.is_user_configured());
    
    // Config with only name should be valid (email optional)
    config.user.name = Some("Valid User".to_string());
    assert!(config.is_user_configured());
    
    // Config with name and email should be valid
    config.user.email = Some("valid@example.com".to_string());
    assert!(config.is_user_configured());
    
    // Config with name and empty email should still be valid
    config.user.email = Some("".to_string());
    assert!(config.is_user_configured());
}
