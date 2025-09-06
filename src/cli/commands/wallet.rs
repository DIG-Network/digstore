//! Wallet management commands

use crate::cli::{context::CliContext, WalletCommands};
use crate::config::{ConfigKey, ConfigValue, GlobalConfig};
use crate::core::error::{DigstoreError, Result};
use crate::wallet::WalletManager;
use colored::Colorize;
use dialoguer::Confirm;
use dig_wallet::Wallet;
use serde_json::json;

/// Execute wallet command
pub fn execute(command: WalletCommands) -> Result<()> {
    match command {
        WalletCommands::List { json } => execute_list(json),
        WalletCommands::Info { profile, json, show_mnemonic } => execute_info(profile, json, show_mnemonic),
        WalletCommands::Create { profile, from_mnemonic, set_active, json } => {
            execute_create(profile, from_mnemonic, set_active, json)
        },
        WalletCommands::Delete { profile, force, json } => execute_delete(profile, force, json),
        WalletCommands::SetActive { profile, json } => execute_set_active(profile, json),
        WalletCommands::Active { json } => execute_active(json),
        WalletCommands::Export { profile, json } => execute_export(profile, json),
    }
}

/// List all wallets
fn execute_list(json: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    let wallets = rt.block_on(async {
        Wallet::list_wallets().await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to list wallets: {}", e),
    })?;

    let config = GlobalConfig::load().unwrap_or_default();
    let active_profile = config.wallet.active_profile.as_deref().unwrap_or("default");

    if json {
        let output = json!({
            "wallets": wallets,
            "active_profile": active_profile
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "Wallet Profiles".cyan().bold());
        println!("{}", "═══════════════".cyan());
        
        if wallets.is_empty() {
            println!("No wallets found.");
            println!("Use 'digstore wallet create <profile>' to create a new wallet.");
        } else {
            for wallet in &wallets {
                if wallet == active_profile {
                    println!("  {} {}", wallet.green(), "(active)".bright_black());
                } else {
                    println!("  {}", wallet);
                }
            }
            
            println!();
            println!("Active profile: {}", active_profile.green());
            println!("Total wallets: {}", wallets.len());
        }
    }

    Ok(())
}

/// Show wallet information
fn execute_info(profile: Option<String>, json: bool, show_mnemonic: bool) -> Result<()> {
    let config = GlobalConfig::load().unwrap_or_default();
    let cli_profile = CliContext::get_wallet_profile();
    let wallet_profile = profile.as_deref()
        .or_else(|| cli_profile.as_deref())
        .or(config.wallet.active_profile.as_deref())
        .unwrap_or("default");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    let wallet = rt.block_on(async {
        Wallet::load(Some(wallet_profile.to_string()), false).await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to load wallet '{}': {}", wallet_profile, e),
    })?;

    let public_key = rt.block_on(async {
        wallet.get_public_synthetic_key().await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to get public key: {}", e),
    })?;

    let address = rt.block_on(async {
        wallet.get_owner_public_key().await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to get address: {}", e),
    })?;

    let mnemonic = if show_mnemonic {
        Some(wallet.get_mnemonic().map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to get mnemonic: {}", e),
        })?)
    } else {
        None
    };

    if json {
        let mut output = json!({
            "profile": wallet_profile,
            "wallet_name": wallet.get_wallet_name(),
            "public_key": hex::encode(public_key.to_bytes()),
            "address": address,
            "is_active": config.wallet.active_profile.as_deref() == Some(wallet_profile)
        });

        if let Some(mnemonic) = mnemonic {
            output.as_object_mut().unwrap().insert("mnemonic".to_string(), json!(mnemonic));
        }

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "Wallet Information".cyan().bold());
        println!("{}", "══════════════════".cyan());
        println!("Profile: {}", wallet_profile.green());
        println!("Name: {}", wallet.get_wallet_name());
        println!("Public Key: {}", hex::encode(public_key.to_bytes()));
        println!("Address: {}", address.bright_blue());
        
        if config.wallet.active_profile.as_deref() == Some(wallet_profile) {
            println!("Status: {} {}", "Active".green().bold(), "✓".green());
        } else {
            println!("Status: Inactive");
        }

        if let Some(mnemonic) = mnemonic {
            println!();
            println!("{}", "MNEMONIC PHRASE (KEEP SECURE):".red().bold());
            println!("{}", mnemonic.bright_white().on_black());
        }
    }

    Ok(())
}

/// Create a new wallet
fn execute_create(profile: String, from_mnemonic: Option<String>, set_active: bool, json: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    // Check if wallet already exists
    let existing_wallets = rt.block_on(async {
        Wallet::list_wallets().await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to list wallets: {}", e),
    })?;

    if existing_wallets.contains(&profile) {
        return Err(DigstoreError::ConfigurationError {
            reason: format!("Wallet profile '{}' already exists", profile),
        });
    }

    let was_imported = from_mnemonic.is_some();
    let mnemonic = if let Some(mnemonic) = from_mnemonic {
        // Import from provided mnemonic
        rt.block_on(async {
            Wallet::import_wallet(&profile, Some(&mnemonic)).await
        }).map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to import wallet: {}", e),
        })?;
        mnemonic
    } else {
        // Generate new wallet
        rt.block_on(async {
            Wallet::create_new_wallet(&profile).await
        }).map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create wallet: {}", e),
        })?
    };

    // Set as active if requested
    if set_active {
        let mut config = GlobalConfig::load().unwrap_or_default();
        config.wallet.active_profile = Some(profile.clone());
        config.save()?;
    }

    if json {
        let output = json!({
            "profile": profile,
            "mnemonic": mnemonic,
            "active": set_active,
            "imported": was_imported
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if was_imported {
            println!("{}", "✓ Wallet imported successfully!".green().bold());
        } else {
            println!("{}", "✓ Wallet created successfully!".green().bold());
            println!();
            println!("{}", "IMPORTANT: Please write down your mnemonic phrase:".red().bold());
            println!();
            println!("  {}", mnemonic.bright_white().on_black());
            println!();
            println!("{}", "Keep this phrase safe and secure. You'll need it to recover your wallet.".yellow());
        }
        
        println!("Profile: {}", profile.green());
        if set_active {
            println!("Status: {} {}", "Active".green().bold(), "✓".green());
        }
    }

    Ok(())
}

/// Delete a wallet
fn execute_delete(profile: String, force: bool, json: bool) -> Result<()> {
    let config = GlobalConfig::load().unwrap_or_default();
    
    // Don't allow deleting the active profile without confirmation
    if config.wallet.active_profile.as_deref() == Some(&profile) && !force {
        if !json {
            println!("{}", "Warning: You are about to delete the active wallet profile.".yellow().bold());
            println!("This will permanently delete the wallet and all its data.");
            println!();
        }
        
        let confirmed = if json {
            false // JSON mode requires --force flag
        } else {
            Confirm::new()
                .with_prompt(&format!("Are you sure you want to delete wallet '{}'?", profile))
                .default(false)
                .interact()
                .map_err(|e| DigstoreError::ConfigurationError {
                    reason: format!("Failed to get user confirmation: {}", e),
                })?
        };

        if !confirmed {
            if json {
                let output = json!({
                    "error": "Deletion cancelled. Use --force to delete without confirmation."
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Deletion cancelled.");
            }
            return Ok(());
        }
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    let deleted = rt.block_on(async {
        Wallet::delete_wallet(&profile).await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to delete wallet: {}", e),
    })?;

    if !deleted {
        if json {
            let output = json!({
                "error": format!("Wallet profile '{}' not found", profile)
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("Wallet profile '{}' not found.", profile);
        }
        return Ok(());
    }

    // If we deleted the active profile, clear it from config
    if config.wallet.active_profile.as_deref() == Some(&profile) {
        let mut updated_config = config;
        updated_config.wallet.active_profile = None;
        updated_config.save()?;
    }

    if json {
        let output = json!({
            "profile": profile,
            "deleted": true
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", format!("✓ Wallet profile '{}' deleted successfully.", profile).green());
    }

    Ok(())
}

/// Set active wallet profile
fn execute_set_active(profile: String, json: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    // Verify wallet exists
    let wallets = rt.block_on(async {
        Wallet::list_wallets().await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to list wallets: {}", e),
    })?;

    if !wallets.contains(&profile) {
        return Err(DigstoreError::ConfigurationError {
            reason: format!("Wallet profile '{}' does not exist", profile),
        });
    }

    let mut config = GlobalConfig::load().unwrap_or_default();
    config.wallet.active_profile = Some(profile.clone());
    config.save()?;

    if json {
        let output = json!({
            "active_profile": profile
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", format!("✓ Active wallet profile set to '{}'", profile).green());
    }

    Ok(())
}

/// Show active wallet profile
fn execute_active(json: bool) -> Result<()> {
    let config = GlobalConfig::load().unwrap_or_default();
    let active_profile = config.wallet.active_profile.as_deref().unwrap_or("default");

    if json {
        let output = json!({
            "active_profile": active_profile
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Active wallet profile: {}", active_profile.green());
    }

    Ok(())
}

/// Export wallet mnemonic
fn execute_export(profile: Option<String>, json: bool) -> Result<()> {
    let config = GlobalConfig::load().unwrap_or_default();
    let cli_profile = CliContext::get_wallet_profile();
    let wallet_profile = profile.as_deref()
        .or_else(|| cli_profile.as_deref())
        .or(config.wallet.active_profile.as_deref())
        .unwrap_or("default");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| DigstoreError::ConfigurationError {
            reason: format!("Failed to create tokio runtime: {}", e),
        })?;

    let wallet = rt.block_on(async {
        Wallet::load(Some(wallet_profile.to_string()), false).await
    }).map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to load wallet '{}': {}", wallet_profile, e),
    })?;

    let mnemonic = wallet.get_mnemonic().map_err(|e| DigstoreError::ConfigurationError {
        reason: format!("Failed to get mnemonic: {}", e),
    })?;

    if json {
        let output = json!({
            "profile": wallet_profile,
            "mnemonic": mnemonic
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "WALLET MNEMONIC (KEEP SECURE)".red().bold());
        println!("{}", "═════════════════════════════".red());
        println!("Profile: {}", wallet_profile.green());
        println!();
        println!("{}", mnemonic.bright_white().on_black());
        println!();
        println!("{}", "⚠️  SECURITY WARNING:".yellow().bold());
        println!("• Never share this mnemonic phrase with anyone");
        println!("• Store it in a secure location");
        println!("• Anyone with this phrase can access your wallet");
    }

    Ok(())
}
