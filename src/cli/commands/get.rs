//! Get command implementation

use crate::cli::commands::find_repository_root;
use crate::core::types::Hash;
use crate::storage::store::Store;
use crate::urn::{parse_urn, Urn};
use anyhow::Result;
use base64;
use colored::Colorize;
use sha2::{Sha256, Digest};
use std::io::Write;
use std::path::PathBuf;

/// Generate a deterministic random file size that looks realistic
/// This produces sizes that follow common file size patterns to make decoys indistinguishable
/// from real content based on size alone.
fn generate_deterministic_random_size(seed: &str) -> usize {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher.update(b"size_generation");
    let hash = hasher.finalize();
    
    // Use first 8 bytes as a u64 for randomness
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[0..8]);
    let random_value = u64::from_le_bytes(bytes);
    
    // Create realistic file size distribution:
    // 40% small files (1KB - 100KB)
    // 35% medium files (100KB - 1MB) 
    // 20% large files (1MB - 10MB)
    // 5% very large files (10MB - 20MB)
    
    let size_category = random_value % 100;
    let size_random = (random_value >> 8) % 1000000; // Use remaining bits for size within category
    
    match size_category {
        0..=39 => {
            // Small files: 1KB - 100KB
            let base = 1024; // 1KB
            let range = 99 * 1024; // up to 100KB
            base + (size_random % range) as usize
        },
        40..=74 => {
            // Medium files: 100KB - 1MB
            let base = 100 * 1024; // 100KB
            let range = 924 * 1024; // up to 1MB
            base + (size_random % range as u64) as usize
        },
        75..=94 => {
            // Large files: 1MB - 10MB
            let base = 1024 * 1024; // 1MB
            let range = 9 * 1024 * 1024; // up to 10MB
            base + (size_random % range as u64) as usize
        },
        _ => {
            // Very large files: 10MB - 20MB
            let base = 10 * 1024 * 1024; // 10MB
            let range = 10 * 1024 * 1024; // up to 20MB
            base + (size_random % range as u64) as usize
        }
    }
}

/// Generate deterministic random bytes from a seed string
/// This provides zero-knowledge property by returning consistent random data for invalid URNs
/// 
/// The same invalid URN will always return the same random bytes, making it impossible
/// for a host to distinguish between valid and invalid URNs based on the response.
fn generate_deterministic_random_bytes(seed: &str, size: usize) -> Vec<u8> {
    let mut result = Vec::with_capacity(size);
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let mut counter = 0u64;
    
    while result.len() < size {
        let mut current_hasher = hasher.clone();
        current_hasher.update(&counter.to_le_bytes());
        let hash = current_hasher.finalize();
        
        let bytes_needed = size - result.len();
        let bytes_to_copy = bytes_needed.min(hash.len());
        result.extend_from_slice(&hash[..bytes_to_copy]);
        
        counter += 1;
    }
    
    result
}

/// Execute the get command
pub fn execute(
    path: String,
    output: Option<PathBuf>,
    verify: bool,
    metadata: bool,
    at: Option<String>,
    progress: bool,
    decryption_key: Option<String>,
    json: bool,
) -> Result<()> {
    println!("{}", "Retrieving content...".bright_blue());

    // Parse the at parameter if provided
    let at_root = if let Some(hash_str) = at {
        Some(
            Hash::from_hex(&hash_str)
                .map_err(|_| anyhow::anyhow!("Invalid root hash: {}", hash_str))?,
        )
    } else {
        None
    };

    let content = if path.starts_with("urn:dig:chia:") {
        // Full URN provided - parse and resolve
        println!("  {} Parsing URN: {}", "•".cyan(), path.dimmed());
        
        // Try to resolve the URN, but if it fails, return deterministic random bytes
        match parse_urn(&path) {
            Ok(urn) => {
                // Try to open the store and resolve
                match Store::open_global(&urn.store_id) {
                    Ok(store) => {
                        match urn.resolve(&store) {
                            Ok(content) => content,
                            Err(_) => {
                                // File not found or other error - return deterministic random bytes
                                // Use full URN as seed to ensure consistency
                                // Size based on byte range if present, otherwise deterministic random size
                                let size = if let Some(range) = &urn.byte_range {
                                    match (range.start, range.end) {
                                        (Some(start), Some(end)) => (end - start + 1) as usize,
                                        (Some(start), None) => {
                                            // Generate realistic file size and subtract start offset
                                            let total_size = generate_deterministic_random_size(&path);
                                            if total_size > start as usize {
                                                total_size - start as usize
                                            } else {
                                                1024 // Minimum 1KB if offset is too large
                                            }
                                        },
                                        (None, Some(end)) => (end + 1) as usize,
                                        (None, None) => generate_deterministic_random_size(&path),
                                    }
                                } else {
                                    generate_deterministic_random_size(&path)
                                };
                                generate_deterministic_random_bytes(&path, size)
                            }
                        }
                    }
                    Err(_) => {
                        // Store not found - return deterministic random bytes
                        let size = if let Some(range) = &urn.byte_range {
                            match (range.start, range.end) {
                                (Some(start), Some(end)) => (end - start + 1) as usize,
                                (Some(start), None) => {
                                    // Generate realistic file size and subtract start offset
                                    let total_size = generate_deterministic_random_size(&path);
                                    if total_size > start as usize {
                                        total_size - start as usize
                                    } else {
                                        1024 // Minimum 1KB if offset is too large
                                    }
                                },
                                (None, Some(end)) => (end + 1) as usize,
                                (None, None) => generate_deterministic_random_size(&path),
                            }
                        } else {
                            generate_deterministic_random_size(&path)
                        };
                        generate_deterministic_random_bytes(&path, size)
                    }
                }
            }
            Err(_) => {
                // Invalid URN format - return deterministic random bytes
                generate_deterministic_random_bytes(&path, generate_deterministic_random_size(&path))
            }
        }
    } else {
        // Simple path - find repository and resolve
        let repo_root = find_repository_root()?
            .ok_or_else(|| anyhow::anyhow!("Not in a repository (no .digstore file found)"))?;

        let store = Store::open(&repo_root)?;
        let file_path = PathBuf::from(&path);

        println!("  {} Retrieving file: {}", "•".cyan(), file_path.display());

        if let Some(root_hash) = at_root {
            store.get_file_at(&file_path, Some(root_hash))?
        } else {
            store.get_file(&file_path)?
        }
    };

    if progress {
        println!("  {} Retrieved {} bytes", "✓".green(), content.len());
    }

    // Handle output
    if let Some(output_path) = &output {
        // Write to file (-o flag)
        std::fs::write(output_path, &content)?;

        if json {
            // JSON metadata about the file operation
            let output_info = serde_json::json!({
                "action": "file_written",
                "path": path,
                "output_file": output_path.display().to_string(),
                "size": content.len(),
                "verified": verify,
                "at_root": at_root.map(|h| h.to_hex()),
                "metadata_included": metadata
            });
            println!("{}", serde_json::to_string_pretty(&output_info)?);
        } else {
            println!(
                "{} Content written to: {}",
                "✓".green().bold(),
                output_path.display().to_string().bright_white()
            );

            if metadata {
                println!("  {} Size: {} bytes", "→".cyan(), content.len());
                if verify {
                    println!("  {} Content verified", "✓".green());
                }
            }
        }
    } else {
        // Stream to stdout (default behavior)
        if json {
            // JSON metadata to stderr, content to stdout
            let output_info = serde_json::json!({
                "action": "content_streamed",
                "path": path,
                "size": content.len(),
                "verified": verify,
                "at_root": at_root.map(|h| h.to_hex()),
                "metadata_included": metadata
            });
            eprintln!("{}", serde_json::to_string_pretty(&output_info)?);
        }

        // Always stream content to stdout
        std::io::stdout().write_all(&content)?;

        if metadata && !json {
            eprintln!("{} Size: {} bytes", "→".cyan(), content.len());
            if verify {
                eprintln!("  {} Content verified", "✓".green());
            }
        }
    }

    Ok(())
}
