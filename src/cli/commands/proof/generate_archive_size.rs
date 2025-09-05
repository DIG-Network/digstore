//! Generate archive size proof command (moved from prove_archive_size.rs)

use crate::cli::commands::find_repository_root;
use crate::config::global_config::{GlobalConfig, ConfigKey, ConfigValue};
use crate::core::digstore_file::DigstoreFile;
use crate::core::types::Hash;
use crate::proofs::size_proof::ArchiveSizeProof;
use crate::storage::{Store, dig_archive::get_archive_path};
use anyhow::Result;
use colored::Colorize;
use std::io::Write;
use std::path::PathBuf;

/// Execute the proof generate-archive-size command
pub fn execute(
    store_id: Option<String>,
    output: Option<PathBuf>,
    format: Option<String>,
    verbose: bool,
    show_compression: bool,
    json: bool,
) -> Result<()> {
    println!("{}", "Generating tamper-proof archive size proof...".bright_blue());
    
    // Determine store ID (auto-detect from .digstore file or use provided)
    let (store_id_string, store_id_hash) = match store_id {
        Some(provided_store_id) => {
            if verbose {
                println!("  {} Using provided store ID: {}", "•".cyan(), provided_store_id);
            }
            let parsed_hash = Hash::from_hex(&provided_store_id)
                .map_err(|_| anyhow::anyhow!("Invalid store ID format: {}", provided_store_id))?;
            (provided_store_id, parsed_hash)
        }
        None => {
            if verbose {
                println!("  {} Auto-detecting store ID from .digstore file...", "•".cyan());
            }
            
            // Find .digstore file in current directory or parent directories
            let repo_root = find_repository_root()
                .map_err(|e| anyhow::anyhow!("Failed to search for repository: {}", e))?
                .ok_or_else(|| anyhow::anyhow!(
                    "No .digstore file found in current directory or parent directories.\n  \
                     Either provide store ID as argument or run from a digstore repository directory."
                ))?;
            
            // Load .digstore file to get store ID
            let digstore_file_path = repo_root.join(".digstore");
            let digstore_file = DigstoreFile::load(&digstore_file_path)
                .map_err(|e| anyhow::anyhow!("Failed to load .digstore file: {}", e))?;
            
            let detected_store_id = digstore_file.store_id.clone();
            let parsed_hash = digstore_file.get_store_id()
                .map_err(|e| anyhow::anyhow!("Invalid store ID in .digstore file: {}", e))?;
            
            if verbose {
                println!("  {} Detected store ID: {}", "✓".green(), detected_store_id);
                if let Some(repo_name) = &digstore_file.repository_name {
                    println!("  {} Repository: {}", "•".cyan(), repo_name);
                }
            }
            
            (detected_store_id, parsed_hash)
        }
    };
    
    if verbose {
        println!("  {} Auto-discovering parameters from .dig directory...", "•".cyan());
    }
    
    // Load global configuration to get publisher's public key
    let global_config = GlobalConfig::load().map_err(|e| {
        anyhow::anyhow!("Failed to load global configuration: {}. Please set crypto.public_key using 'digstore config crypto.public_key <hex_key>'", e)
    })?;
    
    let publisher_public_key = match global_config.get(&ConfigKey::CryptoPublicKey) {
        Some(ConfigValue::String(pubkey)) => pubkey,
        _ => {
            return Err(anyhow::anyhow!(
                "Publisher public key not configured. Please set it using:\n  digstore config crypto.public_key <32-byte-hex-key>"
            ));
        }
    };
    
    if verbose {
        println!("  {} Publisher public key: {}...", "•".cyan(), &publisher_public_key[..16]);
        println!("  {} Final store ID: {}", "•".cyan(), store_id_string);
    }
    
    // Open the store to get current root hash and archive size
    let store = Store::open_global(&store_id_hash).map_err(|e| {
        anyhow::anyhow!("Failed to open store {}: {}. Ensure the store exists in ~/.dig/", store_id_string, e)
    })?;
    
    let current_root_hash = store.current_root.ok_or_else(|| {
        anyhow::anyhow!("Store {} has no commits yet. Please commit some data first.", store_id_string)
    })?;
    
    // Get the actual archive file size
    let archive_path = get_archive_path(&store_id_hash)?;
    let actual_file_size = std::fs::metadata(&archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to get archive file size: {}", e))?
        .len();
    
    if verbose {
        println!("  {} Current root hash: {}", "•".cyan(), current_root_hash.to_hex());
        println!("  {} Archive file size: {} bytes", "•".cyan(), actual_file_size);
        println!();
    }
    
    // Generate the proof with auto-discovered parameters
    let proof = match ArchiveSizeProof::generate(&store_id_hash, &current_root_hash, actual_file_size) {
        Ok(mut proof) => {
            // Include publisher's public key in the proof for verification
            proof.publisher_public_key = Some(publisher_public_key.clone());
            
            if verbose {
                println!("  {} Archive located and verified", "✓".green());
                println!("  {} Layer count: {}", "•".cyan(), proof.verified_layer_count);
                println!("  {} Calculated size: {} bytes", "•".cyan(), proof.calculated_total_size);
                println!("  {} Publisher key included in proof", "•".cyan());
            }
            proof
        }
        Err(e) => {
            eprintln!("{} Failed to generate proof: {}", "✗".red(), e);
            return Err(e.into());
        }
    };
    
    // Convert to compressed format
    let hex_output = proof.to_compressed_hex()?;
    
    if show_compression {
        println!();
        println!("{}", "Compression Statistics:".bright_yellow());
        println!("  {} Original proof data: ~{} bytes", "•".cyan(), std::mem::size_of_val(&proof));
        println!("  {} Hex encoded: {} characters", "•".cyan(), hex_output.len());
        println!("  {} Compression achieved through binary encoding", "•".cyan());
        println!();
    }
    
    // Output the proof
    match output {
        Some(output_path) => {
            // Write to file
            if json {
                let json_output = serde_json::to_string_pretty(&proof)?;
                std::fs::write(&output_path, json_output)?;
                println!("{} Proof written to: {}", "✓".green(), output_path.display());
            } else {
                std::fs::write(&output_path, &hex_output)?;
                println!("{} Compressed proof written to: {}", "✓".green(), output_path.display());
            }
        }
        None => {
            // Write to stdout
            if json {
                let json_output = serde_json::to_string_pretty(&proof)?;
                println!("{}", json_output);
            } else {
                println!("{}", hex_output);
            }
        }
    }
    
    if verbose && !json {
        println!();
        println!("{} Archive size proof generated successfully!", "✓".green());
        println!("  {} Proof is tamper-proof and cryptographically secure", "•".cyan());
        println!("  {} Verifier can validate without accessing the archive file", "•".cyan());
    }
    
    Ok(())
}
