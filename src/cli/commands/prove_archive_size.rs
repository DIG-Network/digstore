//! Archive size proof generation command

use crate::core::types::Hash;
use crate::proofs::size_proof::ArchiveSizeProof;
use anyhow::Result;
use colored::Colorize;
use std::io::Write;
use std::path::PathBuf;

/// Execute the prove-archive-size command
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
        println!("  {} Store ID: {}", "•".cyan(), store_id.dimmed());
        println!("  {} Root Hash: {}", "•".cyan(), root_hash.dimmed());
        println!("  {} Expected Size: {} bytes", "•".cyan(), expected_size);
    }
    
    // Generate the tamper-proof proof
    println!("  {} Locating archive and verifying parameters...", "•".cyan());
    let proof = ArchiveSizeProof::generate(&store_id_hash, &root_hash_hash, expected_size)?;
    
    if verbose {
        println!("  {} Archive found and rootHash verified", "✓".green());
        println!("  {} Read layer index ({} layers)", "✓".green(), proof.verified_layer_count);
        println!("  {} Calculated total size: {} bytes", "✓".green(), proof.calculated_total_size);
        println!("  {} Built layer size merkle tree", "✓".green());
        println!("  {} Generated integrity proofs", "✓".green());
    }
    
    // Determine output format
    let output_format = format.as_deref().unwrap_or("compressed");
    
    let proof_data = match output_format {
        "json" => {
            if json {
                serde_json::to_string_pretty(&proof)?
            } else {
                serde_json::to_string_pretty(&serde_json::json!({
                    "proof": proof,
                    "format": "json",
                    "size_bytes": serde_json::to_string(&proof)?.len()
                }))?
            }
        },
        "compressed" | "hex" => {
            let compressed_hex = proof.to_compressed_hex()?;
            
            if show_compression {
                let original_size = serde_json::to_string(&proof)?.len();
                let compressed_size = compressed_hex.len() / 2; // Hex is 2 chars per byte
                let compression_ratio = (original_size - compressed_size) as f64 / original_size as f64 * 100.0;
                
                println!();
                println!("{}", "Compression Statistics:".bold());
                println!("  Original JSON: {} bytes", original_size);
                println!("  Compressed Binary: {} bytes", compressed_size);
                println!("  Hex Encoded: {} characters", compressed_hex.len());
                println!("  Compression Ratio: {:.1}%", compression_ratio);
                println!("  Bandwidth Savings: {:.6}%", 
                    (expected_size - compressed_size as u64) as f64 / expected_size as f64 * 100.0);
                println!();
            }
            
            compressed_hex
        },
        "binary" => {
            return Err(anyhow::anyhow!("Binary format requires --output file (not text-safe)"));
        },
        _ => {
            return Err(anyhow::anyhow!("Invalid format: {}. Use: json, compressed, binary", output_format));
        }
    };
    
    // Handle output
    if let Some(output_path) = &output {
        std::fs::write(output_path, &proof_data)?;
        
        if json {
            let output_info = serde_json::json!({
                "action": "proof_generated",
                "store_id": store_id,
                "root_hash": root_hash,
                "expected_size": expected_size,
                "output_file": output_path.display().to_string(),
                "format": output_format,
                "proof_size": proof_data.len()
            });
            println!("{}", serde_json::to_string_pretty(&output_info)?);
        } else {
            println!(
                "{} Archive size proof written to: {}",
                "✓".green().bold(),
                output_path.display().to_string().bright_white()
            );
            println!("  {} Format: {}", "→".cyan(), output_format);
            println!("  {} Size: {} {}", "→".cyan(), proof_data.len(), 
                if output_format == "compressed" { "characters" } else { "bytes" });
        }
    } else {
        // Output to stdout (default behavior)
        if json {
            let output_info = serde_json::json!({
                "action": "proof_generated", 
                "store_id": store_id,
                "root_hash": root_hash,
                "expected_size": expected_size,
                "format": output_format,
                "proof_size": proof_data.len(),
                "proof": if output_format == "json" { 
                    serde_json::from_str::<serde_json::Value>(&proof_data)? 
                } else { 
                    serde_json::Value::String(proof_data.clone()) 
                }
            });
            eprintln!("{}", serde_json::to_string_pretty(&output_info)?);
        }
        
        // Always output proof data to stdout
        print!("{}", proof_data);
        std::io::stdout().flush()?;
        
        if !json && verbose {
            eprintln!();
            eprintln!("{} Archive size proof generated", "✓".green());
            eprintln!("  {} Format: {}", "→".cyan(), output_format);
            eprintln!("  {} Size: {} {}", "→".cyan(), proof_data.len(),
                if output_format == "compressed" { "characters" } else { "bytes" });
        }
    }
    
    Ok(())
}
