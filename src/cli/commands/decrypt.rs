//! Decrypt command implementation

use crate::cli::commands::find_repository_root;
use crate::core::types::Hash;
use crate::storage::store::Store;
use crate::urn::{parse_urn, Urn};
use anyhow::Result;
use colored::Colorize;
use std::io::Write;
use std::path::PathBuf;

/// Execute the decrypt command
pub fn execute(
    path: String,
    output: Option<PathBuf>,
    urn: Option<String>,
    json: bool,
) -> Result<()> {
    println!("{}", "Decrypting content...".bright_blue());

    // Read the encrypted file from input path
    let encrypted_data = std::fs::read(&path)?;

    println!("  {} Read {} bytes of encrypted data", "•".cyan(), encrypted_data.len());

    // Determine the URN to use for decryption
    let decryption_urn = if let Some(provided_urn) = urn {
        // Use the provided URN
        provided_urn
    } else if path.starts_with("urn:dig:chia:") {
        // The path itself is a URN
        path.clone()
    } else {
        // Try to construct a URN from the file path
        let repo_root = find_repository_root()?
            .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;
        
        let store = Store::open(&repo_root)?;
        let file_path = PathBuf::from(&path);
        
        // Construct URN from store ID and file path
        format!(
            "urn:dig:chia:{}/{}",
            store.store_id.to_hex(),
            file_path.to_string_lossy().replace('\\', "/")
        )
    };

    println!("  {} Using URN for decryption: {}", "•".cyan(), decryption_urn.dimmed());

    // Decrypt the data
    let decrypted_data = crate::crypto::decrypt_data(&encrypted_data, &decryption_urn)?;

    println!("  {} Decrypted {} bytes", "✓".green(), decrypted_data.len());

    // Handle output
    if let Some(output_path) = &output {
        // Write to file (-o flag)
        std::fs::write(output_path, &decrypted_data)?;

        if json {
            // JSON metadata about the operation
            let output_info = serde_json::json!({
                "action": "file_decrypted",
                "input": path,
                "output_file": output_path.display().to_string(),
                "encrypted_size": encrypted_data.len(),
                "decrypted_size": decrypted_data.len(),
                "urn": decryption_urn,
            });
            println!("{}", serde_json::to_string_pretty(&output_info)?);
        } else {
            println!(
                "{} Decrypted content written to: {}",
                "✓".green().bold(),
                output_path.display().to_string().bright_white()
            );
        }
    } else {
        // Stream to stdout (default behavior)
        if json {
            // JSON metadata to stderr, content to stdout
            let output_info = serde_json::json!({
                "action": "content_decrypted",
                "input": path,
                "encrypted_size": encrypted_data.len(),
                "decrypted_size": decrypted_data.len(),
                "urn": decryption_urn,
            });
            eprintln!("{}", serde_json::to_string_pretty(&output_info)?);
        }

        // Stream decrypted content to stdout
        std::io::stdout().write_all(&decrypted_data)?;
    }

    Ok(())
}
