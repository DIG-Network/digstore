//! Verify archive size proof command (moved from verify_archive_size.rs)

use crate::core::types::Hash;
use crate::proofs::size_proof::{verify_compressed_hex_proof, ArchiveSizeProof};
use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

/// Execute the proof verify-archive-size command
pub fn execute(
    proof_input: String,
    store_id: String,
    root_hash: String,
    expected_size: u64,
    expected_publisher_public_key: String,
    from_file: bool,
    verbose: bool,
    json: bool,
) -> Result<()> {
    println!("{}", "Verifying archive size proof...".bright_blue());

    // Parse and validate all parameters
    let store_id_hash = Hash::from_hex(&store_id)
        .map_err(|_| anyhow::anyhow!("Invalid store ID format: {}", store_id))?;
    let root_hash_hash = Hash::from_hex(&root_hash)
        .map_err(|_| anyhow::anyhow!("Invalid root hash format: {}", root_hash))?;

    // Validate publisher public key format
    if expected_publisher_public_key.len() != 64 {
        return Err(anyhow::anyhow!(
            "Publisher public key must be 64 hex characters, got {}",
            expected_publisher_public_key.len()
        ));
    }
    hex::decode(&expected_publisher_public_key).map_err(|_| {
        anyhow::anyhow!(
            "Invalid publisher public key format: {}",
            expected_publisher_public_key
        )
    })?;

    if verbose {
        println!("  {} Store ID: {}", "•".cyan(), store_id);
        println!("  {} Root Hash: {}", "•".cyan(), root_hash);
        println!("  {} Expected Size: {} bytes", "•".cyan(), expected_size);
        println!(
            "  {} Expected Publisher: {}...",
            "•".cyan(),
            &expected_publisher_public_key[..16]
        );
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
        println!(
            "  {} Proof length: {} characters",
            "•".cyan(),
            proof_data.len()
        );
        println!(
            "  {} Verifying proof integrity and parameters...",
            "•".cyan()
        );
    }

    // Verify the proof (try ultra-compressed format first, then fall back to hex)
    let verification_result = if proof_data.contains(':') {
        // Ultra-compressed format with encoding prefix
        let ultra_proof =
            crate::proofs::ultra_compressed_proof::UltraCompressedProof::from_compressed_text(
                &proof_data,
            )?;
        let archive_proof = ultra_proof.to_archive_proof()?;
        crate::proofs::size_proof::verify_archive_size_proof(
            &archive_proof,
            &store_id_hash,
            &root_hash_hash,
            expected_size,
            &expected_publisher_public_key,
        )
    } else {
        // Standard hex format
        verify_compressed_hex_proof(
            &proof_data,
            &store_id_hash,
            &root_hash_hash,
            expected_size,
            &expected_publisher_public_key,
        )
    };

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
                        "publisher_verified": true,
                        "expected_publisher": expected_publisher_public_key,
                        "message": "Archive size proof verified successfully"
                    });
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!();
                    println!("{} Archive size proof verified successfully!", "✓".green());
                    println!("  {} Store ID: {}", "✓".green(), store_id);
                    println!("  {} Root Hash: {}", "✓".green(), root_hash);
                    println!("  {} Size: {} bytes", "✓".green(), expected_size);
                    println!(
                        "  {} Publisher: {}...",
                        "✓".green(),
                        &expected_publisher_public_key[..16]
                    );
                    if verbose {
                        println!();
                        println!(
                            "{} Cryptographically verified: Archive is exactly {} bytes",
                            "🔒".cyan(),
                            expected_size
                        );
                        println!(
                            "  {} Proof is tamper-proof and mathematically sound",
                            "•".cyan()
                        );
                        println!(
                            "  {} Publisher identity verified and matches expected",
                            "•".cyan()
                        );
                        println!(
                            "  {} No file access was required for verification",
                            "•".cyan()
                        );
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
                    println!(
                        "  {} The proof does not match the provided parameters",
                        "•".red()
                    );
                    println!("  {} Possible issues:", "•".red());
                    println!("    {} Wrong store ID, root hash, or size", "•".red());
                    println!(
                        "    {} Wrong publisher public key (proof not from expected publisher)",
                        "•".red()
                    );
                    println!("    {} Corrupted or invalid proof data", "•".red());
                }
                return Err(anyhow::anyhow!("Proof verification failed"));
            }
        },
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
        },
    }

    Ok(())
}
