//! Initialize command implementation

use anyhow::Result;
use crate::storage::store::Store;
use crate::core::types::Hash;
use std::env;
use colored::Colorize;

/// Execute the init command
pub fn execute(
    store_id: Option<String>,
    name: Option<String>,
    no_compression: bool,
    chunk_size: u32,
) -> Result<()> {
    let current_dir = env::current_dir()?;
    
    println!("{}", "Initializing repository...".bright_blue());
    
    // Check if custom store ID was provided
    let actual_store_id = if let Some(id_str) = store_id {
        println!("  {} Using provided store ID: {}", "•".cyan(), id_str.dimmed());
        Hash::from_hex(&id_str)
            .map_err(|_| anyhow::anyhow!("Invalid store ID format: {}", id_str))?
    } else {
        println!("  {} Generating new store ID...", "•".cyan());
        crate::storage::store::generate_store_id()
    };
    
    // Initialize the store
    let store = Store::init(&current_dir)?;
    
    println!("  {} Created store directory: {}", 
        "✓".green(), 
        store.global_path().display().to_string().dimmed());
    
    println!("  {} Generated store ID: {}", 
        "✓".green(), 
        store.store_id().to_hex().bright_cyan());
    
    println!("  {} Created .digstore file", "✓".green());
    println!("  {} Initialized empty repository", "✓".green());
    
    if let Some(repo_name) = name {
        println!("  {} Repository name: {}", "•".cyan(), repo_name.bright_white());
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
    println!("Location: {}", store.global_path().display().to_string().dimmed());
    
    Ok(())
}
