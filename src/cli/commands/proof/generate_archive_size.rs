//! Generate archive size proof command (moved from prove_archive_size.rs)

use crate::core::types::Hash;
use crate::proofs::size_proof::ArchiveSizeProof;
use anyhow::Result;
use colored::Colorize;
use std::io::Write;
use std::path::PathBuf;

/// Execute the proof generate-archive-size command
pub fn execute(
    store_id: String,
    root_hash: String,
    expected_size: u64,
    output: Option<PathBuf>,
    format: Option<String>,
    verbose: bool,
    show_compression: bool,
    json: bool,
) -> Result<()> {
    println!("{}", "Generating tamper-proof archive size proof...".bright_blue());
    
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
    
    // Generate the proof
    let proof = match ArchiveSizeProof::generate(&store_id_hash, &root_hash_hash, expected_size) {
        Ok(proof) => {
            if verbose {
                println!("  {} Archive located and verified", "✓".green());
                println!("  {} Layer count: {}", "•".cyan(), proof.verified_layer_count);
                println!("  {} Calculated size: {} bytes", "•".cyan(), proof.calculated_total_size);
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
