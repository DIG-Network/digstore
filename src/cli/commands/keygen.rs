//! Key generation command implementation

use crate::cli::context::CliContext;
use crate::config::GlobalConfig;
use crate::crypto::{PublicKey, transform_urn, derive_key_from_urn, derive_storage_address};
use crate::wallet::WalletManager;
use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

/// Execute the keygen command
pub fn execute(
    urn: String,
    output: Option<PathBuf>,
    storage_address: bool,
    encryption_key: bool,
    json: bool,
) -> Result<()> {
    execute_with_profile(urn, output, storage_address, encryption_key, json, None)
}

/// Execute the keygen command with optional wallet profile
pub fn execute_with_profile(
    urn: String,
    output: Option<PathBuf>,
    storage_address: bool,
    encryption_key: bool,
    json: bool,
    wallet_profile: Option<String>,
) -> Result<()> {
    println!("{}", "Generating content key...".bright_blue());

    // Get wallet profile from CLI context if not provided
    let effective_wallet_profile = wallet_profile.or_else(|| CliContext::get_wallet_profile());
    
    // Get public key from specified wallet profile or active wallet
    let public_key = WalletManager::get_wallet_public_key(effective_wallet_profile)
        .map_err(|e| anyhow::anyhow!("Failed to get public key from wallet: {}. Ensure you have a wallet initialized.", e))?;
    
    let public_key_hex = public_key.to_hex();
    
    println!("  {} URN: {}", "•".cyan(), urn.dimmed());
    println!("  {} Public Key: {}", "•".cyan(), public_key_hex.dimmed());

    // Generate transformed address for storage
    let transformed_address = transform_urn(&urn, &public_key)?;
    println!("  {} Transformed Address: {}", "•".cyan(), transformed_address.dimmed());

    // Generate storage address from transformed URN
    let storage_addr = derive_storage_address(&urn, &public_key)?;
    
    // Generate encryption key from original URN
    let encryption_key_bytes = derive_key_from_urn(&urn);
    let encryption_key_hex = hex::encode(encryption_key_bytes);

    if json {
        let output_data = serde_json::json!({
            "urn": urn,
            "public_key": public_key_hex,
            "transformed_address": transformed_address,
            "storage_address": storage_addr,
            "encryption_key": encryption_key_hex,
            "key_derivation": "SHA256(urn)",
            "address_derivation": "SHA256(transform(urn + public_key))"
        });
        
        if let Some(output_path) = &output {
            std::fs::write(output_path, serde_json::to_string_pretty(&output_data)?)?;
            println!("  {} Key information written to: {}", "✓".green(), output_path.display());
        } else {
            println!("{}", serde_json::to_string_pretty(&output_data)?);
        }
    } else {
        println!();
        println!("{}", "Generated Keys:".green().bold());
        println!("{}", "═".repeat(50));
        
        if storage_address || (!storage_address && !encryption_key) {
            println!("\n{}", "Storage Address:".bold());
            println!("  Address: {}", storage_addr.bright_cyan());
            println!("  Purpose: Where encrypted data is stored");
            println!("  Derivation: SHA256(transform(URN + public_key))");
        }
        
        if encryption_key || (!storage_address && !encryption_key) {
            println!("\n{}", "Encryption Key:".bold());
            println!("  Key: {}", encryption_key_hex.bright_yellow());
            println!("  Purpose: Encrypt/decrypt data using AES-256-GCM");
            println!("  Derivation: SHA256(URN)");
        }

        println!("\n{}", "URN Transformation:".bold());
        println!("  Original URN: {}", urn.bright_white());
        println!("  Transformed:  {}", transformed_address.bright_magenta());
        println!("  Purpose: Zero-knowledge storage addressing");

        if let Some(output_path) = &output {
            let output_content = format!(
                "URN: {}\nPublic Key: {}\nTransformed Address: {}\nStorage Address: {}\nEncryption Key: {}\n",
                urn, public_key_hex, transformed_address, storage_addr, encryption_key_hex
            );
            std::fs::write(output_path, output_content)?;
            println!("\n  {} Key information written to: {}", "✓".green(), output_path.display());
        }
    }

    println!();
    println!("{}", "Security Properties:".yellow().bold());
    println!("  • Storage layer cannot determine actual URN being used");
    println!("  • Storage layer cannot decrypt data without original URN");
    println!("  • Different public keys create different storage addresses");
    println!("  • Same URN+key always produces same addresses (deterministic)");

    Ok(())
}
