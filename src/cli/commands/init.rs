//! Initialize command implementation

use crate::core::types::Hash;
use crate::storage::store::Store;
use anyhow::Result;
use colored::Colorize;
use std::env;

/// Execute the init command
pub fn execute(
    store_id: Option<String>,
    name: Option<String>,
    no_compression: bool,
    chunk_size: u32,
    encryption_key: Option<String>,
) -> Result<()> {
    let current_dir = env::current_dir()?;

    println!("{}", "Initializing repository...".bright_blue());

    // Validate custom encryption key if provided
    if let Some(ref key) = encryption_key {
        // Validate hex format and length
        if hex::decode(key).map_or(true, |bytes| bytes.len() != 32) {
            return Err(anyhow::anyhow!("Encryption key must be 32 bytes (64 hex characters)"));
        }
        println!("  {} Using custom encryption key for store-wide secret storage", "🔐".cyan());
    }

    // Check if custom store ID was provided
    let actual_store_id = if let Some(id_str) = store_id {
        println!(
            "  {} Using provided store ID: {}",
            "•".cyan(),
            id_str.dimmed()
        );
        Hash::from_hex(&id_str)
            .map_err(|_| anyhow::anyhow!("Invalid store ID format: {}", id_str))?
    } else {
        println!("  {} Generating new store ID...", "•".cyan());
        crate::storage::store::generate_store_id()
    };

    // Initialize the store
    let store = Store::init(&current_dir)?;
    
    // Save custom encryption key to store config if provided
    if let Some(custom_key) = encryption_key {
        let mut store_config = crate::config::StoreConfig::load(&store.store_id)?;
        store_config.set_custom_encryption_key(Some(custom_key));
        store_config.name = name.clone();
        store_config.created_at = Some(chrono::Utc::now().to_rfc3339());
        store_config.save(&store.store_id)?;
        
        println!("  {} Saved custom encryption key to store configuration", "✓".green());
    }

    println!(
        "  {} Created store directory: {}",
        "✓".green(),
        store.global_path().display().to_string().dimmed()
    );

    println!(
        "  {} Generated store ID: {}",
        "✓".green(),
        store.store_id().to_hex().bright_cyan()
    );

    println!("  {} Created .digstore file", "✓".green());
    println!("  {} Initialized empty repository", "✓".green());

    if let Some(repo_name) = name {
        println!(
            "  {} Repository name: {}",
            "•".cyan(),
            repo_name.bright_white()
        );
    }

    if !no_compression {
        println!("  {} Compression: enabled (zstd)", "•".cyan());
    } else {
        println!("  {} Compression: disabled", "•".cyan());
    }

    println!("  {} Chunk size: {}KB", "•".cyan(), chunk_size);

    println!();
    println!("{}", "Repository initialized".green());
    println!("Store ID: {}", store.store_id().to_hex().bright_cyan());
    println!(
        "Location: {}",
        store.global_path().display().to_string().dimmed()
    );

    Ok(())
}
