//! Archive size proof verification command

use crate::core::types::Hash;
use crate::proofs::size_proof::{ArchiveSizeProof, verify_compressed_hex_proof};
use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

/// Execute the verify-archive-size command
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
        println!("  {} Store ID: {}", "•".cyan(), store_id.dimmed());
        println!("  {} Root Hash: {}", "•".cyan(), root_hash.dimmed());
        println!("  {} Expected Size: {} bytes", "•".cyan(), expected_size);
    }
    
    // Read proof data
    let proof_data = if from_file {
        println!("  {} Reading proof from file: {}", "•".cyan(), proof_input);
        std::fs::read_to_string(&proof_input)?
    } else {
        // Assume proof_input is the compressed hex string
        proof_input
    };
    
    let proof_data = proof_data.trim();
    
    if verbose {
        println!("  {} Proof size: {} characters", "•".cyan(), proof_data.len());
        println!("  {} Decompressing and parsing proof...", "•".cyan());
    }
    
    // Verify the proof
    let verification_result = if proof_data.starts_with('{') {
        // JSON format proof
        let proof: ArchiveSizeProof = serde_json::from_str(proof_data)?;
        crate::proofs::size_proof::verify_archive_size_proof(
            &proof, 
            &store_id_hash, 
            &root_hash_hash, 
            expected_size
        )?
    } else {
        // Compressed hex format (default)
        verify_compressed_hex_proof(
            proof_data,
            &store_id_hash,
            &root_hash_hash, 
            expected_size
        )?
    };
    
    if verification_result {
        if json {
            let result = serde_json::json!({
                "verification": "successful",
                "store_id": store_id,
                "root_hash": root_hash,
                "verified_size": expected_size,
                "proof_format": if proof_data.starts_with('{') { "json" } else { "compressed" },
                "proof_size": proof_data.len(),
                "timestamp": chrono::Utc::now().timestamp()
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!();
            println!("{} Archive size proof verified successfully!", "✓".green().bold());
            println!("  {} Store ID: {} ✓", "→".cyan(), store_id.bright_cyan());
            println!("  {} Root Hash: {} ✓", "→".cyan(), root_hash.bright_cyan());
            println!("  {} Verified Size: {} bytes ✓", "→".cyan(), expected_size.to_string().bright_white());
            
            if verbose {
                println!("  {} Proof Format: {}", "→".cyan(), 
                    if proof_data.starts_with('{') { "JSON" } else { "Compressed Binary Hex" });
                println!("  {} Proof Size: {} {}", "→".cyan(), proof_data.len(),
                    if proof_data.starts_with('{') { "bytes" } else { "characters" });
                
                // Calculate bandwidth savings
                let bandwidth_savings = (expected_size - proof_data.len() as u64) as f64 / expected_size as f64 * 100.0;
                println!("  {} Bandwidth Savings: {:.6}%", "→".cyan(), bandwidth_savings);
            }
            
            println!();
            println!("{}", "🔒 Cryptographically verified: Storage provider has the exact archive".green());
            println!("   for the specified repository state without any possibility of deception.");
        }
    } else {
        if json {
            let result = serde_json::json!({
                "verification": "failed",
                "store_id": store_id,
                "root_hash": root_hash,
                "expected_size": expected_size,
                "error": "Proof verification failed",
                "timestamp": chrono::Utc::now().timestamp()
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!();
            println!("{} Archive size proof verification failed!", "✗".red().bold());
            println!("  {} The proof is invalid or has been tampered with", "→".red());
            println!("  {} Possible causes:", "→".yellow());
            println!("    • Wrong store ID or root hash provided");
            println!("    • Proof was generated for different parameters");
            println!("    • Proof has been corrupted or tampered with");
            println!("    • Archive size doesn't match claimed size");
        }
        
        return Err(anyhow::anyhow!("Proof verification failed"));
    }
    
    Ok(())
}
