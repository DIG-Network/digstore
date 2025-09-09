//! Global configuration management
//!
//! Provides Git-like global configuration stored in ~/.dig/config.toml

use crate::core::error::{DigstoreError, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Global configuration for Digstore
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// User configuration
    pub user: UserConfig,
    /// Core configuration
    pub core: CoreConfig,
    /// Crypto configuration
    pub crypto: CryptoConfig,
    /// Wallet configuration
    pub wallet: WalletConfig,
    /// Custom configuration values
    #[serde(flatten)]
    pub custom: HashMap<String, ConfigValue>,
}

/// User-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// User's name for commits
    pub name: Option<String>,
    /// User's email for commits
    pub email: Option<String>,
}

/// Core Digstore configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    /// Default chunk size
    pub chunk_size: Option<u32>,
    /// Default compression
    pub compression: Option<String>,
    /// Editor for commit messages
    pub editor: Option<String>,
}

/// Crypto configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoConfig {
    /// Public key for URN transformation (hex encoded)
    pub public_key: Option<String>,
    /// Enable encrypted storage (always true for zero-knowledge properties)
    pub encrypted_storage: Option<bool>,
}

/// Wallet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Active wallet profile name
    pub active_profile: Option<String>,
}

/// Configuration value types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    String(String),
    Number(i64),
    Boolean(bool),
}

/// Configuration key for setting values
#[derive(Debug, Clone)]
pub enum ConfigKey {
    UserName,
    UserEmail,
    CoreEditor,
    CoreChunkSize,
    CoreCompression,
    CryptoPublicKey,
    CryptoEncryptedStorage,
    WalletActiveProfile,
    Custom(String),
}

impl ConfigKey {
    pub fn from_str(key: &str) -> Option<Self> {
        match key {
            "user.name" => Some(ConfigKey::UserName),
            "user.email" => Some(ConfigKey::UserEmail),
            "core.editor" => Some(ConfigKey::CoreEditor),
            "core.chunk_size" => Some(ConfigKey::CoreChunkSize),
            "core.compression" => Some(ConfigKey::CoreCompression),
            "crypto.public_key" => Some(ConfigKey::CryptoPublicKey),
            "crypto.encrypted_storage" => Some(ConfigKey::CryptoEncryptedStorage),
            "wallet.active_profile" => Some(ConfigKey::WalletActiveProfile),
            _ => Some(ConfigKey::Custom(key.to_string())),
        }
    }

    pub fn to_str(&self) -> &str {
        match self {
            ConfigKey::UserName => "user.name",
            ConfigKey::UserEmail => "user.email",
            ConfigKey::CoreEditor => "core.editor",
            ConfigKey::CoreChunkSize => "core.chunk_size",
            ConfigKey::CoreCompression => "core.compression",
            ConfigKey::CryptoPublicKey => "crypto.public_key",
            ConfigKey::CryptoEncryptedStorage => "crypto.encrypted_storage",
            ConfigKey::WalletActiveProfile => "wallet.active_profile",
            ConfigKey::Custom(key) => key,
        }
    }
}

impl GlobalConfig {
    /// Load global configuration from disk
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;

        if !config_path.exists() {
            // Return default configuration if file doesn't exist
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: GlobalConfig =
            toml::from_str(&content).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to parse global config: {}", e),
            })?;

        Ok(config)
    }

    /// Save global configuration to disk
    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content =
            toml::to_string_pretty(self).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to serialize config: {}", e),
            })?;

        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the path to the global configuration file
    pub fn get_config_path() -> Result<PathBuf> {
        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;

        let dig_dir = user_dirs.home_dir().join(".dig");
        Ok(dig_dir.join("config.toml"))
    }

    /// Get a configuration value
    pub fn get(&self, key: &ConfigKey) -> Option<ConfigValue> {
        match key {
            ConfigKey::UserName => self
                .user
                .name
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::UserEmail => self
                .user
                .email
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::CoreEditor => self
                .core
                .editor
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::CoreChunkSize => self
                .core
                .chunk_size
                .as_ref()
                .map(|n| ConfigValue::Number(*n as i64)),
            ConfigKey::CoreCompression => self
                .core
                .compression
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::CryptoPublicKey => self
                .crypto
                .public_key
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::CryptoEncryptedStorage => {
                self.crypto.encrypted_storage.map(ConfigValue::Boolean)
            },
            ConfigKey::WalletActiveProfile => self
                .wallet
                .active_profile
                .as_ref()
                .map(|s| ConfigValue::String(s.clone())),
            ConfigKey::Custom(key) => self.custom.get(key).cloned(),
        }
    }

    /// Set a configuration value
    pub fn set(&mut self, key: ConfigKey, value: ConfigValue) -> Result<()> {
        match key {
            ConfigKey::UserName => {
                if let ConfigValue::String(name) = value {
                    self.user.name = Some(name);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "user.name must be a string".to_string(),
                    });
                }
            },
            ConfigKey::UserEmail => {
                if let ConfigValue::String(email) = value {
                    self.user.email = Some(email);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "user.email must be a string".to_string(),
                    });
                }
            },
            ConfigKey::CoreEditor => {
                if let ConfigValue::String(editor) = value {
                    self.core.editor = Some(editor);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "core.editor must be a string".to_string(),
                    });
                }
            },
            ConfigKey::CoreChunkSize => {
                if let ConfigValue::Number(size) = value {
                    self.core.chunk_size = Some(size as u32);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "core.chunk_size must be a number".to_string(),
                    });
                }
            },
            ConfigKey::CoreCompression => {
                if let ConfigValue::String(compression) = value {
                    self.core.compression = Some(compression);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "core.compression must be a string".to_string(),
                    });
                }
            },
            ConfigKey::CryptoPublicKey => {
                if let ConfigValue::String(pubkey) = value {
                    // Validate it's a valid hex string
                    if pubkey.len() != 64 || hex::decode(&pubkey).is_err() {
                        return Err(DigstoreError::ConfigurationError {
                            reason:
                                "crypto.public_key must be a 64-character hex string (32 bytes)"
                                    .to_string(),
                        });
                    }
                    self.crypto.public_key = Some(pubkey);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "crypto.public_key must be a string".to_string(),
                    });
                }
            },
            ConfigKey::CryptoEncryptedStorage => {
                if let ConfigValue::Boolean(enabled) = value {
                    self.crypto.encrypted_storage = Some(enabled);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "crypto.encrypted_storage must be a boolean".to_string(),
                    });
                }
            },
            ConfigKey::WalletActiveProfile => {
                if let ConfigValue::String(profile) = value {
                    self.wallet.active_profile = Some(profile);
                } else {
                    return Err(DigstoreError::ConfigurationError {
                        reason: "wallet.active_profile must be a string".to_string(),
                    });
                }
            },
            ConfigKey::Custom(key_name) => {
                self.custom.insert(key_name, value);
            },
        }
        Ok(())
    }

    /// Unset a configuration value
    pub fn unset(&mut self, key: &ConfigKey) {
        match key {
            ConfigKey::UserName => self.user.name = None,
            ConfigKey::UserEmail => self.user.email = None,
            ConfigKey::CoreEditor => self.core.editor = None,
            ConfigKey::CoreChunkSize => self.core.chunk_size = None,
            ConfigKey::CoreCompression => self.core.compression = None,
            ConfigKey::CryptoPublicKey => self.crypto.public_key = None,
            ConfigKey::CryptoEncryptedStorage => self.crypto.encrypted_storage = None,
            ConfigKey::WalletActiveProfile => self.wallet.active_profile = None,
            ConfigKey::Custom(key_name) => {
                self.custom.remove(key_name);
            },
        }
    }

    /// List all configuration values
    pub fn list(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();

        if let Some(name) = &self.user.name {
            entries.push(("user.name".to_string(), name.clone()));
        }
        if let Some(email) = &self.user.email {
            entries.push(("user.email".to_string(), email.clone()));
        }
        if let Some(editor) = &self.core.editor {
            entries.push(("core.editor".to_string(), editor.clone()));
        }
        if let Some(chunk_size) = &self.core.chunk_size {
            entries.push(("core.chunk_size".to_string(), chunk_size.to_string()));
        }
        if let Some(compression) = &self.core.compression {
            entries.push(("core.compression".to_string(), compression.clone()));
        }
        if let Some(public_key) = &self.crypto.public_key {
            entries.push(("crypto.public_key".to_string(), public_key.clone()));
        }
        if let Some(encrypted_storage) = self.crypto.encrypted_storage {
            entries.push((
                "crypto.encrypted_storage".to_string(),
                encrypted_storage.to_string(),
            ));
        }
        if let Some(active_profile) = &self.wallet.active_profile {
            entries.push(("wallet.active_profile".to_string(), active_profile.clone()));
        }

        for (key, value) in &self.custom {
            let value_str = match value {
                ConfigValue::String(s) => s.clone(),
                ConfigValue::Number(n) => n.to_string(),
                ConfigValue::Boolean(b) => b.to_string(),
            };
            entries.push((key.clone(), value_str));
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Get author name with fallback
    pub fn get_author_name(&self) -> String {
        self.user.name.clone().unwrap_or_else(|| {
            // Try environment variables as fallback
            std::env::var("GIT_AUTHOR_NAME")
                .or_else(|_| std::env::var("USER"))
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "Unknown".to_string())
        })
    }

    /// Get author email with fallback
    pub fn get_author_email(&self) -> Option<String> {
        self.user
            .email
            .clone()
            .or_else(|| std::env::var("GIT_AUTHOR_EMAIL").ok())
    }

    /// Check if user configuration is complete
    pub fn is_user_configured(&self) -> bool {
        self.user.name.is_some() // Only name is required, email is optional
    }

    /// Prompt user to configure if not set
    pub fn ensure_user_configured(&mut self) -> Result<()> {
        if self.is_user_configured() {
            return Ok(());
        }

        use colored::Colorize;
        use dialoguer::Input;

        println!();
        println!("{}", "User configuration required".yellow().bold());
        println!("Please configure your identity for commits:");
        println!();

        // Get name if not set
        if self.user.name.is_none() {
            let default_name = std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "".to_string());

            let name: String = if crate::cli::context::CliContext::is_non_interactive() {
                default_name // Use default in non-interactive mode
            } else {
                Input::new()
                    .with_prompt("Your name")
                    .default(default_name)
                    .interact_text()
                    .map_err(|e| DigstoreError::ConfigurationError {
                        reason: format!("Failed to get user input: {}", e),
                    })?
            };

            self.user.name = Some(name);
        }

        // Get email if not set (optional) - only ask once
        if self.user.email.is_none() {
            let email: String = if crate::cli::context::CliContext::is_non_interactive() {
                "".to_string() // Use empty string in non-interactive mode
            } else {
                Input::new()
                    .with_prompt("Your email (optional)")
                    .allow_empty(true)
                    .interact_text()
                    .map_err(|e| DigstoreError::ConfigurationError {
                        reason: format!("Failed to get user input: {}", e),
                    })?
            };

            // Always set email to mark that we asked (empty string means user chose not to provide)
            self.user.email = Some(email);
        }

        // Save configuration
        self.save()?;

        println!();
        println!("{}", "✓ Configuration saved".green());
        println!("  Name: {}", self.user.name.as_ref().unwrap().cyan());
        let email_display = self.user.email.as_ref().unwrap();
        if email_display.is_empty() {
            println!("  Email: {}", "(not set)".dimmed());
        } else {
            println!("  Email: {}", email_display.cyan());
        }
        println!();
        println!("You can change these settings anytime with:");
        println!("  {}", "digstore config user.name \"Your Name\"".cyan());
        println!(
            "  {}",
            "digstore config user.email \"your@email.com\"".cyan()
        );
        println!();

        Ok(())
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            user: UserConfig {
                name: None,
                email: None,
            },
            core: CoreConfig {
                chunk_size: Some(1024), // 1KB default
                compression: Some("zstd".to_string()),
                editor: None,
            },
            crypto: CryptoConfig {
                public_key: None,
                encrypted_storage: Some(true),
            },
            wallet: WalletConfig {
                active_profile: Some("default".to_string()),
            },
            custom: HashMap::new(),
        }
    }
}

impl ConfigValue {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ConfigValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            ConfigValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            ConfigValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_creation_and_defaults() {
        let config = GlobalConfig::default();

        assert!(config.user.name.is_none());
        assert!(config.user.email.is_none());
        assert_eq!(config.core.chunk_size, Some(1024));
        assert_eq!(config.core.compression, Some("zstd".to_string()));
        assert!(!config.is_user_configured());
    }

    #[test]
    fn test_config_set_and_get() -> Result<()> {
        let mut config = GlobalConfig::default();

        // Set user name
        config.set(
            ConfigKey::UserName,
            ConfigValue::String("Test User".to_string()),
        )?;

        // Set user email
        config.set(
            ConfigKey::UserEmail,
            ConfigValue::String("test@example.com".to_string()),
        )?;

        // Verify values
        assert_eq!(config.user.name, Some("Test User".to_string()));
        assert_eq!(config.user.email, Some("test@example.com".to_string()));
        assert!(config.is_user_configured());

        Ok(())
    }

    #[test]
    fn test_config_list() -> Result<()> {
        let mut config = GlobalConfig::default();

        config.set(
            ConfigKey::UserName,
            ConfigValue::String("John Doe".to_string()),
        )?;
        config.set(
            ConfigKey::UserEmail,
            ConfigValue::String("john@example.com".to_string()),
        )?;

        let entries = config.list();

        assert!(entries
            .iter()
            .any(|(k, v)| k == "user.name" && v == "John Doe"));
        assert!(entries
            .iter()
            .any(|(k, v)| k == "user.email" && v == "john@example.com"));

        Ok(())
    }

    #[test]
    fn test_config_key_parsing() {
        assert!(matches!(
            ConfigKey::from_str("user.name"),
            Some(ConfigKey::UserName)
        ));
        assert!(matches!(
            ConfigKey::from_str("user.email"),
            Some(ConfigKey::UserEmail)
        ));
        assert!(matches!(
            ConfigKey::from_str("custom.key"),
            Some(ConfigKey::Custom(_))
        ));
    }
}
