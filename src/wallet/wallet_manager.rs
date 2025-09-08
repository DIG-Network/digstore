//! Wallet management and initialization
//!
//! Provides wallet initialization checks and user prompts for mnemonic management

use crate::config::GlobalConfig;
use crate::core::error::{DigstoreError, Result};
use crate::crypto::PublicKey;
use colored::Colorize;
use dialoguer::{Confirm, Input, Select};
use dig_wallet::Wallet;

/// Wallet status enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum WalletStatus {
    /// Wallet is initialized and ready
    Initialized,
    /// Wallet is not initialized
    NotInitialized,
    /// Wallet exists but has issues
    Corrupted,
}

/// Wallet manager for handling wallet initialization and checks
pub struct WalletManager {
    wallet_name: String,
}

impl WalletManager {
    /// Create a new wallet manager with default profile
    pub fn new() -> Result<Self> {
        let config = GlobalConfig::load().unwrap_or_default();
        let wallet_name = config
            .wallet
            .active_profile
            .unwrap_or_else(|| "default".to_string());

        Ok(Self { wallet_name })
    }

    /// Create a new wallet manager with specific profile
    pub fn new_with_profile(profile: Option<String>) -> Result<Self> {
        let wallet_name = if let Some(profile) = profile {
            profile
        } else {
            let config = GlobalConfig::load().unwrap_or_default();
            config
                .wallet
                .active_profile
                .unwrap_or_else(|| "default".to_string())
        };

        Ok(Self { wallet_name })
    }

    /// Check the current wallet status
    pub fn check_status(&self) -> WalletStatus {
        // Ensure .dig directory exists before checking wallet status
        if self.ensure_dig_directory_exists().is_err() {
            return WalletStatus::NotInitialized;
        }

        // Proactively check for and fix keyring issues
        if let Err(_) = self.verify_keyring_health() {
            // If keyring verification fails, try cleanup
            let _ = self.manual_wallet_cleanup();
        }

        // Use tokio runtime to check if wallet exists
        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        });

        match rt {
            Ok(runtime) => {
                match runtime
                    .block_on(async { Wallet::load(Some(self.wallet_name.clone()), false).await })
                {
                    Ok(_) => WalletStatus::Initialized,
                    Err(dig_wallet::WalletError::WalletNotFound(_)) => {
                        // Always return NotInitialized for WalletNotFound
                        WalletStatus::NotInitialized
                    },
                    Err(_) => {
                        // Check if this is first time user to avoid "corrupted" message
                        if Self::is_first_time_user() {
                            WalletStatus::NotInitialized
                        } else {
                            WalletStatus::Corrupted
                        }
                    },
                }
            },
            Err(_) => WalletStatus::NotInitialized,
        }
    }

    /// Ensure wallet is initialized, prompting user if necessary
    pub fn ensure_wallet_initialized(&self) -> Result<()> {
        match self.check_status() {
            WalletStatus::Initialized => Ok(()),
            WalletStatus::NotInitialized => self.prompt_wallet_initialization(),
            WalletStatus::Corrupted => self.handle_corrupted_wallet(),
        }
    }

    /// Ensure .dig directory exists and keyring file is properly initialized
    fn ensure_dig_directory_exists(&self) -> Result<()> {
        use directories::UserDirs;

        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
        let dig_dir = user_dirs.home_dir().join(".dig");

        // Create .dig directory if it doesn't exist
        if !dig_dir.exists() {
            std::fs::create_dir_all(&dig_dir)?;
        }

        // Ensure keyring.yaml exists and is properly formatted
        let keyring_path = dig_dir.join("keyring.yaml");
        if !keyring_path.exists() {
            // Create empty keyring file with proper structure
            let empty_keyring = "wallets: {}\n";
            std::fs::write(&keyring_path, empty_keyring)?;
        } else {
            // Check if existing keyring is malformed and fix it if needed
            match std::fs::read_to_string(&keyring_path) {
                Ok(content) => {
                    let trimmed = content.trim();
                    if trimmed.is_empty() || trimmed == "wallets:" {
                        // Fix malformed keyring
                        let proper_keyring = "wallets: {}\n";
                        std::fs::write(&keyring_path, proper_keyring)?;
                    } else {
                        // Try to parse as YAML and validate structure
                        match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                            Ok(yaml) => {
                                // Check if it has the expected structure
                                if !yaml.is_mapping()
                                    || !yaml.get("wallets").map_or(false, |w| w.is_mapping())
                                {
                                    // Invalid structure, replace with proper format
                                    let proper_keyring = "wallets: {}\n";
                                    std::fs::write(&keyring_path, proper_keyring)?;
                                }
                            },
                            Err(_) => {
                                // YAML is invalid, replace with proper structure
                                let proper_keyring = "wallets: {}\n";
                                std::fs::write(&keyring_path, proper_keyring)?;
                            },
                        }
                    }
                },
                Err(_) => {
                    // Can't read file, recreate it
                    let empty_keyring = "wallets: {}\n";
                    std::fs::write(&keyring_path, empty_keyring)?;
                },
            }
        }

        Ok(())
    }

    /// Verify keyring file health and fix any issues
    fn verify_keyring_health(&self) -> Result<()> {
        use directories::UserDirs;

        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
        let dig_dir = user_dirs.home_dir().join(".dig");
        let keyring_path = dig_dir.join("keyring.yaml");

        if keyring_path.exists() {
            match std::fs::read_to_string(&keyring_path) {
                Ok(content) => {
                    // Test if the keyring can be parsed as valid YAML
                    if let Err(_) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                        // Malformed YAML - recreate completely
                        return Err(DigstoreError::ConfigurationError {
                            reason: "Keyring file is malformed and needs to be recreated"
                                .to_string(),
                        });
                    } else {
                        // YAML is valid, but check if it has basic structure we expect
                        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                            if !yaml.is_mapping() || yaml.get("wallets").is_none() {
                                // Missing expected structure - needs recreation
                                return Err(DigstoreError::ConfigurationError {
                                    reason: "Keyring file has unexpected structure and needs to be recreated".to_string(),
                                });
                            }
                        }
                    }
                },
                Err(_) => {
                    // Can't read file - needs recreation
                    return Err(DigstoreError::ConfigurationError {
                        reason: "Keyring file is unreadable and needs to be recreated".to_string(),
                    });
                },
            }
        }

        Ok(())
    }

    /// Check if this is a first-time user (no wallets exist or .dig directory doesn't exist)
    fn is_first_time_user() -> bool {
        use directories::UserDirs;

        // First check if .dig directory exists
        let user_dirs = match UserDirs::new() {
            Some(dirs) => dirs,
            None => return true, // Can't determine home, assume first time
        };

        let dig_dir = user_dirs.home_dir().join(".dig");
        if !dig_dir.exists() {
            return true; // No .dig directory = definitely first time
        }

        // Check if any wallet files exist in .dig directory
        let keyring_path = dig_dir.join("keyring.yaml");
        if !keyring_path.exists() {
            return true; // No keyring file = first time
        }

        // Try to list wallets using dig-wallet API
        let rt = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => {
                // If we can't create runtime, check file system directly
                return !keyring_path.exists();
            },
        };

        match rt.block_on(async { Wallet::list_wallets().await }) {
            Ok(wallets) => wallets.is_empty(),
            Err(_) => {
                // If we can't list wallets but keyring exists, it might be corrupted
                // Check if keyring file is empty or malformed
                match std::fs::read_to_string(&keyring_path) {
                    Ok(content) => content.trim().is_empty() || content.trim() == "wallets:",
                    Err(_) => true, // Can't read file, assume first time
                }
            },
        }
    }

    /// Display welcome message for first-time users
    fn display_welcome_message() {
        println!();
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════╗".bright_cyan()
        );
        println!(
            "{}",
            "║                                                          ║".bright_cyan()
        );
        println!(
            "{}",
            "║                🌐 Welcome to the DIG Network! 🌐         ║".bright_cyan()
        );
        println!(
            "{}",
            "║                                                          ║".bright_cyan()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════╝".bright_cyan()
        );
        println!();
        println!(
            "{}",
            "🚀 You're about to join the decentralized data revolution!"
                .bright_green()
                .bold()
        );
        println!();
        println!("The DIG Network is a decentralized content-addressable storage system");
        println!("that provides zero-knowledge data storage with cryptographic proofs.");
        println!();
        println!("{}", "Key Features:".yellow().bold());
        println!("  🔐 Zero-knowledge storage - node are not able to decrypt your data");
        println!("  🌍 Decentralized network - no single point of failure");
        println!("  🔗 Content-addressable - data integrity guaranteed");
        println!("  📋 Merkle proofs - cryptographically verifiable data");
        println!("  🎯 URN-based retrieval - permanent, reliable addressing");
        println!();
        println!(
            "{}",
            "To get started, you'll need a wallet to manage your cryptographic keys."
                .bright_white()
                .bold()
        );
        println!("Your wallet contains a unique mnemonic phrase that secures your data.");
        println!();
    }

    /// Prompt user for wallet initialization
    fn prompt_wallet_initialization(&self) -> Result<()> {
        // Check if this is a first-time user
        let is_first_time = Self::is_first_time_user();
        if is_first_time {
            Self::display_welcome_message();
        } else {
            println!();
            println!("{}", "Wallet not found".yellow().bold());
            println!("A wallet is required to use digstore. You can either:");
            println!("  1. Generate a new mnemonic phrase (recommended for new users)");
            println!("  2. Import an existing mnemonic phrase");
            println!();
        }

        let options = vec![
            "Generate new mnemonic",
            "Import existing mnemonic",
            "Cancel",
        ];

        let selection = if crate::cli::context::CliContext::is_non_interactive() {
            0 // Default to "Generate new mnemonic" in non-interactive mode
        } else {
            Select::new()
                .with_prompt("What would you like to do?")
                .items(&options)
                .default(0)
                .interact()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to get user input: {}", e),
                })?
        };

        match selection {
            0 => self.generate_new_wallet_with_context(is_first_time),
            1 => self.import_existing_wallet(),
            2 => {
                println!("Operation cancelled.");
                std::process::exit(0);
            },
            _ => unreachable!(),
        }
    }

    /// Generate a new wallet with a fresh mnemonic
    fn generate_new_wallet(&self) -> Result<()> {
        // Determine if first time for context
        let is_first_time = Self::is_first_time_user();
        self.generate_new_wallet_with_context(is_first_time)
    }

    /// Generate a new wallet with context about first-time user status
    fn generate_new_wallet_with_context(&self, is_first_time: bool) -> Result<()> {
        // Ensure .dig directory and keyring exist before wallet creation
        self.ensure_dig_directory_exists()?;

        // Additional safety: verify keyring is readable before proceeding
        self.verify_keyring_health()?;

        println!();
        if is_first_time {
            println!(
                "{}",
                "🎉 Creating your first DIG Network wallet..."
                    .bright_cyan()
                    .bold()
            );
            println!();
            println!("This wallet will be used to:");
            println!("  • Secure your data with zero-knowledge encryption");
            println!("  • Generate unique storage addresses for your content");
            println!("  • Create cryptographic proofs for data integrity");
            println!("  • Access the decentralized DIG Network");
        } else {
            println!("{}", "Generating new wallet...".cyan());
        }

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        let mnemonic = rt
            .block_on(async { Wallet::create_new_wallet(&self.wallet_name).await })
            .map_err(|e| {
                // Check if this is a keyring format issue
                if e.to_string().contains("missing field `wallets`") || e.to_string().contains("Serialization error") {
                    DigstoreError::ConfigurationError {
                        reason: "Wallet configuration is corrupted. The system will attempt to fix this automatically.".to_string(),
                    }
                } else {
                    DigstoreError::ConfigurationError {
                        reason: format!("Failed to generate wallet: {}", e),
                    }
                }
            })?;

        println!();
        println!("{}", "✓ Wallet created successfully!".green().bold());
        println!();
        println!(
            "{}",
            "🔑 IMPORTANT: Please write down your mnemonic phrase:"
                .red()
                .bold()
        );
        println!();
        println!("   ┌─────────────────────────────────────────────────────────────┐");
        println!("   │  {}  │", mnemonic.bright_white().bold());
        println!("   └─────────────────────────────────────────────────────────────┘");
        println!();
        println!("{}", "⚠️  SECURITY NOTICE:".yellow().bold());
        println!("   • This phrase is the ONLY way to recover your wallet");
        println!("   • Please import this key into your Chia Wallet Software");
        println!("   • Never share it with anyone or store it digitally");
        println!("   • Anyone with this phrase can access your data");
        println!();

        // Confirm user has written down the mnemonic
        let confirmed = if crate::cli::context::CliContext::is_non_interactive() {
            true // Auto-confirm in non-interactive mode
        } else {
            Confirm::new()
                .with_prompt("Have you securely written down your mnemonic phrase?")
                .default(false)
                .interact()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to get user confirmation: {}", e),
                })?
        };

        if !confirmed {
            println!();
            println!(
                "{}",
                "⚠️  Please write down your mnemonic phrase before continuing."
                    .yellow()
                    .bold()
            );
            println!();
            println!("   ┌─────────────────────────────────────────────────────────────┐");
            println!("   │  {}  │", mnemonic.bright_white().bold());
            println!("   └─────────────────────────────────────────────────────────────┘");
            println!();
            return Err(DigstoreError::ConfigurationError {
                reason: "User did not confirm mnemonic backup".to_string(),
            });
        }

        println!();
        if is_first_time {
            println!(
                "{}",
                "🎊 Welcome to the DIG Network! Your wallet is ready."
                    .bright_green()
                    .bold()
            );
            println!();
            println!("You can now:");
            println!(
                "  • Initialize repositories with {}",
                "digstore init".cyan()
            );
            println!(
                "  • Add and commit files with {}",
                "digstore add <files>".cyan()
            );
            println!(
                "  • Generate proofs with {}",
                "digstore proof generate".cyan()
            );
            println!("  • Manage wallets with {}", "digstore wallet".cyan());
            println!();
            println!("For help at any time, use: {}", "digstore --help".cyan());
        } else {
            println!("{}", "Wallet setup complete!".green().bold());
        }
        Ok(())
    }

    /// Import an existing wallet from mnemonic
    fn import_existing_wallet(&self) -> Result<()> {
        // Ensure .dig directory and keyring exist before wallet import
        self.ensure_dig_directory_exists()?;

        // Verify keyring health before import
        self.verify_keyring_health()?;

        println!();
        println!("{}", "Import existing wallet".cyan().bold());
        println!("Please enter your mnemonic phrase:");
        println!();

        let mnemonic: String = if crate::cli::context::CliContext::is_non_interactive() {
            return Err(DigstoreError::ConfigurationError {
                reason: "Cannot import wallet in non-interactive mode. Use --auto-import-mnemonic flag instead.".to_string(),
            });
        } else {
            Input::new()
                .with_prompt("Mnemonic phrase")
                .interact_text()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to get mnemonic input: {}", e),
                })?
        };

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        rt.block_on(async { Wallet::import_wallet(&self.wallet_name, Some(&mnemonic)).await })
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Invalid mnemonic phrase: {}", e),
            })?;

        println!();
        println!("{}", "✓ Wallet imported successfully!".green().bold());
        Ok(())
    }

    /// Handle corrupted wallet scenario
    fn handle_corrupted_wallet(&self) -> Result<()> {
        println!();
        println!("{}", "Wallet appears to be corrupted".red().bold());
        println!("This could happen if the wallet file was damaged or modified.");
        println!();

        let options = vec![
            "Try to recover (if you have the mnemonic)",
            "Delete and create new wallet",
            "Cancel",
        ];

        let selection = if crate::cli::context::CliContext::is_non_interactive() {
            1 // Default to "Delete and create new wallet" in non-interactive mode
        } else {
            Select::new()
                .with_prompt("What would you like to do?")
                .items(&options)
                .default(0)
                .interact()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to get user input: {}", e),
                })?
        };

        match selection {
            0 => {
                // Try to recover with existing mnemonic
                self.import_existing_wallet()
            },
            1 => {
                // Confirm deletion
                let confirmed = if crate::cli::context::CliContext::should_auto_accept() {
                    true // Auto-confirm in non-interactive or yes mode
                } else {
                    Confirm::new()
                        .with_prompt("This will permanently delete your current wallet. Are you sure?")
                        .default(false)
                        .interact()
                        .map_err(|e| DigstoreError::ConfigurationError {
                            reason: format!("Failed to get user confirmation: {}", e),
                        })?
                };

                if confirmed {
                    // Try to delete the wallet using dig-wallet API
                    let rt = tokio::runtime::Runtime::new().map_err(|e| {
                        DigstoreError::ConfigurationError {
                            reason: format!("Failed to create tokio runtime: {}", e),
                        }
                    })?;

                    // First try the API deletion
                    let delete_result =
                        rt.block_on(async { Wallet::delete_wallet(&self.wallet_name).await });

                    match delete_result {
                        Ok(_) => {
                            println!("{}", "✓ Wallet deleted successfully".green());
                        },
                        Err(e) => {
                            // API deletion failed - try manual cleanup
                            println!("{}", format!("API deletion failed: {}", e).yellow());
                            println!("{}", "Attempting manual cleanup...".cyan());

                            if let Err(cleanup_err) = self.manual_wallet_cleanup() {
                                println!(
                                    "{}",
                                    format!("Manual cleanup also failed: {}", cleanup_err).red()
                                );
                                println!(
                                    "{}",
                                    "Proceeding with new wallet creation anyway...".yellow()
                                );
                            } else {
                                println!("{}", "✓ Manual cleanup successful".green());
                            }
                        },
                    }

                    self.generate_new_wallet()
                } else {
                    println!("Operation cancelled.");
                    std::process::exit(0);
                }
            },
            2 => {
                println!("Operation cancelled.");
                std::process::exit(0);
            },
            _ => unreachable!(),
        }
    }

    /// Manual wallet cleanup when keyring is unreadable or malformed
    fn manual_wallet_cleanup(&self) -> Result<()> {
        use directories::UserDirs;

        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;
        let dig_dir = user_dirs.home_dir().join(".dig");
        let keyring_path = dig_dir.join("keyring.yaml");

        // Completely remove and recreate the keyring file to ensure clean state
        if keyring_path.exists() {
            std::fs::remove_file(&keyring_path)?;
        }

        // Ensure directory exists
        std::fs::create_dir_all(&dig_dir)?;

        // Create a fresh, properly formatted keyring file
        let fresh_keyring = "wallets: {}\n";
        std::fs::write(&keyring_path, fresh_keyring)?;

        Ok(())
    }

    /// Auto-generate wallet without prompts
    pub fn auto_generate_wallet(&self) -> Result<()> {
        // Ensure .dig directory exists first
        self.ensure_dig_directory_exists()?;

        // Verify keyring health before auto-generation
        self.verify_keyring_health()?;

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        // Check if wallet already exists
        match rt.block_on(async { Wallet::load(Some(self.wallet_name.clone()), false).await }) {
            Ok(_) => Ok(()), // Wallet already exists, nothing to do
            Err(dig_wallet::WalletError::WalletNotFound(_)) => {
                // Wallet doesn't exist, create it
                let _mnemonic = rt
                    .block_on(async { Wallet::create_new_wallet(&self.wallet_name).await })
                    .map_err(|e| {
                        // Check if this is a keyring format issue
                        if e.to_string().contains("missing field `wallets`") || e.to_string().contains("Serialization error") {
                            DigstoreError::ConfigurationError {
                                reason: "Wallet configuration is corrupted. The system will attempt to fix this automatically.".to_string(),
                            }
                        } else {
                            DigstoreError::ConfigurationError {
                                reason: format!("Failed to generate wallet: {}", e),
                            }
                        }
                    })?;

                // Set as active profile in config
                let mut config = GlobalConfig::load().unwrap_or_default();
                config.wallet.active_profile = Some(self.wallet_name.clone());
                config.save()?;

                Ok(())
            },
            Err(e) => Err(DigstoreError::ConfigurationError {
                reason: format!("Failed to check wallet status: {}", e),
            }),
        }
    }

    /// Auto-import wallet from mnemonic without prompts
    pub fn auto_import_wallet(&self, mnemonic: &str) -> Result<()> {
        // Ensure .dig directory exists first
        self.ensure_dig_directory_exists()?;

        // Verify keyring health before auto-import
        self.verify_keyring_health()?;

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        // Check if wallet already exists
        match rt.block_on(async { Wallet::load(Some(self.wallet_name.clone()), false).await }) {
            Ok(_) => Ok(()), // Wallet already exists, nothing to do
            Err(dig_wallet::WalletError::WalletNotFound(_)) => {
                // Wallet doesn't exist, import it
                rt.block_on(async {
                    Wallet::import_wallet(&self.wallet_name, Some(mnemonic)).await
                })
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to import wallet: {}", e),
                })?;

                // Set as active profile in config
                let mut config = GlobalConfig::load().unwrap_or_default();
                config.wallet.active_profile = Some(self.wallet_name.clone());
                config.save()?;

                Ok(())
            },
            Err(e) => Err(DigstoreError::ConfigurationError {
                reason: format!("Failed to check wallet status: {}", e),
            }),
        }
    }

    /// Get the wallet instance if initialized
    pub fn get_wallet(&self) -> Result<Wallet> {
        if self.check_status() != WalletStatus::Initialized {
            return Err(DigstoreError::ConfigurationError {
                reason: "Wallet is not initialized".to_string(),
            });
        }

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        rt.block_on(async { Wallet::load(Some(self.wallet_name.clone()), false).await })
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to load wallet: {}", e),
            })
    }

    /// Get the public key from the active wallet or specified profile
    pub fn get_active_wallet_public_key() -> Result<PublicKey> {
        Self::get_wallet_public_key(None)
    }

    /// Get the public key from a specific wallet profile or active wallet
    pub fn get_wallet_public_key(profile: Option<String>) -> Result<PublicKey> {
        let config = GlobalConfig::load().unwrap_or_default();
        let wallet_name = profile.unwrap_or_else(|| {
            config
                .wallet
                .active_profile
                .unwrap_or_else(|| "default".to_string())
        });

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        let wallet = rt
            .block_on(async { Wallet::load(Some(wallet_name.clone()), false).await })
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!(
                    "Failed to load wallet '{}': {}. Use 'digstore wallet create {}' to create it.",
                    wallet_name, e, wallet_name
                ),
            })?;

        let dig_wallet_public_key = rt
            .block_on(async { wallet.get_public_synthetic_key().await })
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to get public key from wallet: {}", e),
            })?;

        // Convert dig-wallet PublicKey to our crypto::PublicKey
        let public_key_bytes = dig_wallet_public_key.to_bytes().to_vec();
        Ok(PublicKey::new(public_key_bytes, "bls12-381".to_string()))
    }

    /// Get the private key from the active wallet
    pub fn get_active_wallet_private_key() -> Result<dig_wallet::SecretKey> {
        let config = GlobalConfig::load().unwrap_or_default();
        let wallet_name = config
            .wallet
            .active_profile
            .unwrap_or_else(|| "default".to_string());

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        let wallet = rt.block_on(async {
            Wallet::load(Some(wallet_name.clone()), false).await
        }).map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to load active wallet '{}': {}. Use 'digstore wallet create {}' to create it.", wallet_name, e, wallet_name),
        })?;

        rt.block_on(async { wallet.get_private_synthetic_key().await })
            .map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to get private key from wallet: {}", e),
            })
    }

    /// Get the active wallet instance
    pub fn get_active_wallet() -> Result<Wallet> {
        let config = GlobalConfig::load().unwrap_or_default();
        let wallet_name = config
            .wallet
            .active_profile
            .unwrap_or_else(|| "default".to_string());

        let rt = tokio::runtime::Runtime::new().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

        rt.block_on(async {
            Wallet::load(Some(wallet_name.clone()), false).await
        }).map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to load active wallet '{}': {}. Use 'digstore wallet create {}' to create it.", wallet_name, e, wallet_name),
        })
    }
}

impl Default for WalletManager {
    fn default() -> Self {
        Self::new().expect("Failed to create wallet manager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_wallet_status_not_initialized() {
        let manager = WalletManager::new().unwrap();
        // This test would require mocking the dig-wallet API
        // For now, just test that the manager can be created
        assert_eq!(manager.wallet_name, "default");
    }

    #[test]
    fn test_wallet_manager_creation() {
        let manager = WalletManager::new();
        assert!(manager.is_ok());
    }
}
