use crate::core::error::DigstoreError;
use crate::proofs::{Proof, ProofElement, ProofMetadata, ProofPosition, ProofTarget};
use crate::storage::Store;
use crate::urn::parse_urn;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct ProveArgs {
    /// Target to prove (file path or URN)
    #[arg(value_name = "TARGET")]
    pub target: String,

    /// Output file for proof (default: stdout)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Output format: json, binary, text
    #[arg(long, default_value = "json")]
    pub format: String,

    /// Prove at specific commit hash
    #[arg(long)]
    pub at: Option<String>,

    /// Prove specific byte range
    #[arg(long)]
    pub bytes: Option<String>,

    /// Generate compact proof
    #[arg(long)]
    pub compact: bool,
}

pub fn execute(
    target: String,
    output: Option<PathBuf>,
    format: String,
    at: Option<String>,
    bytes: Option<String>,
    compact: bool,
) -> Result<()> {
    let args = ProveArgs {
        target,
        output,
        format,
        at,
        bytes,
        compact,
    };

    let current_dir = std::env::current_dir()?;

    // Try to open store from current directory
    let store = match Store::open(&current_dir) {
        Ok(store) => store,
        Err(_) => {
            // If no local store, try to parse as URN
            if args.target.starts_with("urn:dig:") {
                return handle_urn_prove(&args);
            } else {
                return Err(DigstoreError::store_not_found(current_dir).into());
            }
        }
    };

    // Check if target is a URN
    if args.target.starts_with("urn:dig:") {
        return handle_urn_prove(&args);
    }

    // Handle as file path
    handle_path_prove(&store, &args)
}

fn handle_path_prove(store: &Store, args: &ProveArgs) -> Result<()> {
    println!("{}", "Generating proof...".green());
    println!("  • Target: {}", args.target);

    let file_path = Path::new(&args.target);

    // Parse root hash if provided
    let at_root = if let Some(at_hash) = &args.at {
        let root_hash = crate::core::types::Hash::from_hex(at_hash)
            .map_err(|_| DigstoreError::internal("Invalid hash format"))?;
        Some(root_hash)
    } else {
        store.current_root()
    };

    // Generate the proof
    let proof = if let Some(byte_range_str) = &args.bytes {
        // Parse byte range
        let byte_range =
            crate::urn::parser::parse_byte_range(&format!("#bytes={}", byte_range_str))?;
        println!(
            "  • Byte range: {}-{}",
            byte_range
                .start
                .map(|s| s.to_string())
                .unwrap_or("0".to_string()),
            byte_range
                .end
                .map(|e| e.to_string())
                .unwrap_or("end".to_string())
        );

        Proof::new_byte_range_proof(
            store,
            file_path,
            byte_range.start.unwrap_or(0),
            byte_range.end.unwrap_or(u64::MAX),
            at_root,
        )?
    } else {
        println!("  • Generating file proof");
        Proof::new_file_proof(store, file_path, at_root)?
    };

    if let Some(root) = at_root {
        println!("  • At root: {}", root.to_hex().cyan());
    }

    // Output the proof
    output_proof(&proof, args)?;

    println!("{}", "✓ Proof generated".green());

    Ok(())
}

fn handle_urn_prove(args: &ProveArgs) -> Result<()> {
    println!("{}", "Generating proof from URN...".green());

    let mut urn = parse_urn(&args.target)?;
    println!("  • Store ID: {}", urn.store_id.to_hex().cyan());

    // Add byte range if specified
    if let Some(byte_range_str) = &args.bytes {
        let byte_range =
            crate::urn::parser::parse_byte_range(&format!("#bytes={}", byte_range_str))?;
        println!(
            "  • Byte range: {}-{}",
            byte_range
                .start
                .map(|s| s.to_string())
                .unwrap_or("0".to_string()),
            byte_range
                .end
                .map(|e| e.to_string())
                .unwrap_or("end".to_string())
        );
        urn.byte_range = Some(byte_range);
    }

    // Open the store
    let store = Store::open_global(&urn.store_id)?;

    // Generate proof based on URN
    let proof = if let Some(path) = &urn.resource_path {
        let file_path = Path::new(path);
        if let Some(byte_range) = &urn.byte_range {
            Proof::new_byte_range_proof(
                &store,
                file_path,
                byte_range.start.unwrap_or(0),
                byte_range.end.unwrap_or(u64::MAX),
                urn.root_hash,
            )?
        } else {
            Proof::new_file_proof(&store, file_path, urn.root_hash)?
        }
    } else {
        // Prove entire layer
        let root_hash = urn.root_hash.unwrap_or_else(|| {
            store
                .current_root()
                .unwrap_or_else(|| crate::core::types::Hash::zero())
        });
        Proof::new_layer_proof(&store, root_hash)?
    };

    // Output the proof
    output_proof(&proof, args)?;

    println!("{}", "✓ Proof generated".green());
    Ok(())
}

fn output_proof(proof: &Proof, args: &ProveArgs) -> Result<()> {
    let proof_data = match args.format.as_str() {
        "json" => {
            if args.compact {
                serde_json::to_string(proof)?
            } else {
                proof.to_json()?
            }
        }
        "text" => format_proof_as_text(proof),
        "binary" => {
            // Serialize proof to binary format using bincode
            bincode::serialize(proof)
                .map_err(|e| DigstoreError::Serialization(e))?
                .into_iter()
                .map(|b| b as char)
                .collect::<String>()
        }
        _ => {
            return Err(DigstoreError::internal("Invalid format. Use: json, text, binary").into());
        }
    };

    if let Some(output_path) = &args.output {
        std::fs::write(output_path, proof_data)?;
        println!(
            "  • Proof written to: {}",
            output_path.display().to_string().cyan()
        );
    } else {
        println!("\n{}", "Proof:".bold());
        println!("{}", proof_data);
    }

    Ok(())
}

fn format_proof_as_text(proof: &Proof) -> String {
    let mut output = String::new();

    output.push_str(&format!("Proof Type: {}\n", proof.proof_type));
    output.push_str(&format!("Target: {:?}\n", proof.target));
    output.push_str(&format!("Root Hash: {}\n", proof.root.to_hex()));
    output.push_str(&format!("Generated: {}\n", proof.metadata.timestamp));
    output.push_str(&format!("Store ID: {}\n", proof.metadata.store_id.to_hex()));

    if let Some(layer_num) = proof.metadata.layer_number {
        output.push_str(&format!("Layer: {}\n", layer_num));
    }

    output.push_str(&format!(
        "Proof Elements: {} elements\n",
        proof.proof_path.len()
    ));

    for (i, element) in proof.proof_path.iter().enumerate() {
        output.push_str(&format!(
            "  {}: {} - {}\n",
            i + 1,
            match element.position {
                ProofPosition::Left => "Left",
                ProofPosition::Right => "Right",
            },
            element.hash.to_hex()
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_format_proof_as_text() {
        use crate::core::types::Hash;
        use crate::proofs::{ProofMetadata, ProofPosition, ProofTarget};

        let proof = Proof {
            version: "1.0".to_string(),
            proof_type: "file".to_string(),
            target: ProofTarget::File {
                path: "test.txt".into(),
                file_hash: Hash::zero(),
                at: Some(Hash::zero()),
            },
            root: Hash::zero(),
            proof_path: vec![
                ProofElement {
                    hash: Hash::zero(),
                    position: ProofPosition::Left,
                },
                ProofElement {
                    hash: Hash::zero(),
                    position: ProofPosition::Right,
                },
            ],
            metadata: ProofMetadata {
                timestamp: 1234567890,
                layer_number: Some(1),
                store_id: Hash::zero(),
            },
        };

        let text = format_proof_as_text(&proof);
        assert!(text.contains("Proof Type:"));
        assert!(text.contains("Root Hash:"));
        assert!(text.contains("Proof Elements: 2"));
        assert!(text.contains("Left"));
        assert!(text.contains("Right"));
    }
}
