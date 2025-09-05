//! Verify archive size proof command (moved from verify_archive_size.rs)

use crate::core::types::Hash;
use crate::proofs::size_proof::{ArchiveSizeProof, verify_compressed_hex_proof};
use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

/// Execute the proof verify-archive-size command
pub fn execute(
    proof_input: String,
    store_id: String,
    root_hash: String,
    expected_size: u64,
    from_file: bool,
    verbose: bool,
    json: bool,
) -> Result<()> {
    println!("{}", "Verifying archive size proof...".bright_blue());
    
    // Parse store ID and root hash
    let store_id_hash = Hash::from_hex(&store_id)
        .map_err(|_| anyhow::anyhow!("Invalid store ID format: {}", store_id))?;
    let root_hash_hash = Hash::from_hex(&root_hash)
        .map_err(|_| anyhow::anyhow!("Invalid root hash format: {}", root_hash))?;
    
    if verbose {
        println!("  {} Store ID: {}", "•".cyan(), store_id);
        println!("  {} Root Hash: {}", "•".cyan(), root_hash);
        println!("  {} Expected Size: {} bytes", "•".cyan(), expected_size);
        println!();
    }
    
    // Get proof data
    let proof_data = if from_file {
        if verbose {
            println!("  {} Reading proof from file: {}", "•".cyan(), proof_input);
        }
        std::fs::read_to_string(&proof_input)
            .map_err(|e| anyhow::anyhow!("Failed to read proof file '{}': {}", proof_input, e))?
            .trim()
            .to_string()
    } else {
        if proof_input == "-" {
            if verbose {
                println!("  {} Reading proof from stdin", "•".cyan());
            }
            use std::io::Read;
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            buffer.trim().to_string()
        } else {
            proof_input
        }
    };
    
    if verbose {
        println!("  {} Proof length: {} characters", "•".cyan(), proof_data.len());
        println!("  {} Verifying proof integrity and parameters...", "•".cyan());
    }
    
    // Verify the proof
    let verification_result = verify_compressed_hex_proof(
        &proof_data,
        &store_id_hash,
        &root_hash_hash,
        expected_size,
    );
    
    match verification_result {
        Ok(is_valid) => {
            if is_valid {
                if json {
                    let result = serde_json::json!({
                        "status": "success",
                        "verified": true,
                        "store_id": store_id,
                        "root_hash": root_hash,
                        "expected_size": expected_size,
                        "message": "Archive size proof verified successfully"
                    });
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!();
                    println!("{} Archive size proof verified successfully!", "✓".green());
                    println!("  {} Store ID: {}", "✓".green(), store_id);
                    println!("  {} Root Hash: {}", "✓".green(), root_hash);  
                    println!("  {} Size: {} bytes", "✓".green(), expected_size);
                    if verbose {
                        println!();
                        println!("{} Cryptographically verified: Archive is exactly {} bytes", "🔒".cyan(), expected_size);
                        println!("  {} Proof is tamper-proof and mathematically sound", "•".cyan());
                        println!("  {} No file access was required for verification", "•".cyan());
                    }
                }
            } else {
                if json {
                    let result = serde_json::json!({
                        "status": "error",
                        "verified": false,
                        "store_id": store_id,
                        "root_hash": root_hash,
                        "expected_size": expected_size,
                        "message": "Archive size proof verification failed"
                    });
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!();
                    println!("{} Archive size proof verification failed!", "✗".red());
                    println!("  {} The proof does not match the provided parameters", "•".red());
                    println!("  {} Either the proof is invalid or parameters are incorrect", "•".red());
                }
                return Err(anyhow::anyhow!("Proof verification failed"));
            }
        }
        Err(e) => {
            if json {
                let result = serde_json::json!({
                    "status": "error",
                    "verified": false,
                    "store_id": store_id,
                    "root_hash": root_hash,
                    "expected_size": expected_size,
                    "error": e.to_string(),
                    "message": "Failed to verify archive size proof"
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!();
                println!("{} Failed to verify proof: {}", "✗".red(), e);
            }
            return Err(e.into());
        }
    }
    
    Ok(())
}
