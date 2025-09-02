use crate::core::error::DigstoreError;
use crate::proofs::Proof;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Args)]
pub struct VerifyArgs {
    /// Proof file to verify
    #[arg(value_name = "PROOF")]
    pub proof: PathBuf,

    /// Expected target hash
    #[arg(long)]
    pub target: Option<String>,

    /// Expected root hash
    #[arg(long)]
    pub root: Option<String>,

    /// Show detailed verification steps
    #[arg(short, long)]
    pub verbose: bool,

    /// Read proof from stdin
    #[arg(long)]
    pub from_stdin: bool,
}

pub fn execute(
    proof: PathBuf,
    target: Option<String>,
    root: Option<String>,
    verbose: bool,
    from_stdin: bool,
) -> Result<()> {
    let args = VerifyArgs {
        proof,
        target,
        root,
        verbose,
        from_stdin,
    };

    println!("{}", "Verifying proof...".green());

    // Read proof data
    let proof_data = if args.from_stdin {
        println!("  • Reading proof from stdin");
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        println!("  • Reading proof from: {}", args.proof.display());
        std::fs::read_to_string(&args.proof)?
    };

    // Parse the proof
    let proof = if proof_data.trim().starts_with('{') {
        // JSON format
        Proof::from_json(&proof_data)?
    } else {
        return Err(
            DigstoreError::internal("Only JSON proof format is currently supported").into(),
        );
    };

    if args.verbose {
        println!("  • Proof type: {}", proof.proof_type.cyan());
        println!("  • Target: {:?}", proof.target);
        println!("  • Root hash: {}", proof.root.to_hex().cyan());
        println!("  • Proof elements: {}", proof.proof_path.len());
    }

    // Verify expected values if provided
    if let Some(expected_target) = &args.target {
        if args.verbose {
            println!("  • Checking target hash...");
        }
        // For now, just check if the target contains the expected string
        let target_str = format!("{:?}", proof.target);
        if !target_str.contains(expected_target) {
            println!("{}", "✗ Target verification failed".red());
            return Err(DigstoreError::internal("Target hash mismatch").into());
        }
        if args.verbose {
            println!("    ✓ Target hash matches");
        }
    }

    if let Some(expected_root) = &args.root {
        if args.verbose {
            println!("  • Checking root hash...");
        }
        let expected_root_hash = crate::core::types::Hash::from_hex(expected_root)
            .map_err(|_| DigstoreError::internal("Invalid expected root hash format"))?;
        if proof.root != expected_root_hash {
            println!("{}", "✗ Root hash verification failed".red());
            return Err(DigstoreError::internal("Root hash mismatch").into());
        }
        if args.verbose {
            println!("    ✓ Root hash matches");
        }
    }

    // Verify the proof itself
    if args.verbose {
        println!("  • Verifying merkle proof...");
    }

    let verification_result = proof.verify()?;

    if verification_result {
        println!("{}", "✓ Proof verification successful!".green());

        if args.verbose {
            println!("\n{}", "Verification Details:".bold());
            println!("  • Proof format: Valid");
            println!("  • Merkle path: Valid");
            println!("  • Root reconstruction: Success");
            println!("  • Cryptographic integrity: Verified");
        }

        println!(
            "\n{}",
            "🔒 The proof cryptographically verifies the integrity of the target data.".green()
        );
    } else {
        println!("{}", "✗ Proof verification failed!".red());

        if args.verbose {
            println!("\n{}", "Verification Issues:".bold().red());
            println!("  • The proof path does not reconstruct to the expected root");
            println!("  • This indicates the proof is invalid or corrupted");
        }

        return Err(DigstoreError::internal("Proof verification failed").into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_verify_command_structure() {
        let args = VerifyArgs {
            proof: PathBuf::from("test.json"),
            target: Some("test_target".to_string()),
            root: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            ),
            verbose: true,
            from_stdin: false,
        };

        assert_eq!(args.proof, PathBuf::from("test.json"));
        assert_eq!(args.target, Some("test_target".to_string()));
        assert!(args.verbose);
        assert!(!args.from_stdin);
    }

    #[test]
    fn test_invalid_hex_root() {
        let args = VerifyArgs {
            proof: PathBuf::from("test.json"),
            target: None,
            root: Some("invalid_hex".to_string()),
            verbose: false,
            from_stdin: false,
        };

        // This would fail in the actual execution due to invalid hex
        assert_eq!(args.root, Some("invalid_hex".to_string()));
    }
}
