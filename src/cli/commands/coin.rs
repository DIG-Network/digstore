//! Datastore coin management commands

use crate::cli::context::CliContext;
use crate::core::error::Result;
use crate::datastore_coin::{DatastoreCoinManager, DatastoreId};
use crate::wallet::WalletManager;
use clap::Subcommand;
use colored::Colorize;
use std::path::PathBuf;
use tabled::{Table, Tabled};

#[derive(Debug, Subcommand)]
pub enum CoinCommand {
    /// Create a new datastore coin
    Create {
        /// Datastore ID or path
        datastore: String,
        
        /// Size of the datastore in bytes (if not auto-detected)
        #[arg(long)]
        size: Option<u64>,
        
        /// Wallet profile to use
        #[arg(long, short = 'w')]
        wallet: Option<String>,
    },
    
    /// Mint a pending coin on the blockchain
    Mint {
        /// Coin ID to mint
        coin_id: String,
        
        /// Wallet profile to use
        #[arg(long, short = 'w')]
        wallet: Option<String>,
    },
    
    /// List datastore coins
    List {
        /// Show only active coins
        #[arg(long)]
        active: bool,
        
        /// Filter by owner address
        #[arg(long)]
        owner: Option<String>,
        
        /// Filter by datastore ID
        #[arg(long)]
        datastore: Option<String>,
    },
    
    /// Show detailed information about a coin
    Info {
        /// Coin ID
        coin_id: String,
    },
    
    /// Transfer coin ownership
    Transfer {
        /// Coin ID to transfer
        coin_id: String,
        
        /// Recipient address
        to: String,
        
        /// Wallet profile to use
        #[arg(long, short = 'w')]
        wallet: Option<String>,
    },
    
    /// Spend a coin to release collateral
    Spend {
        /// Coin ID to spend
        coin_id: String,
        
        /// Wallet profile to use
        #[arg(long, short = 'w')]
        wallet: Option<String>,
    },
    
    /// Show coin statistics
    Stats,
    
    /// Calculate collateral requirement
    Collateral {
        /// Size in bytes
        size: u64,
    },
}

pub fn handle_coin_command(ctx: &CliContext, cmd: CoinCommand) -> Result<()> {
    let coin_manager = get_coin_manager()?;
    
    match cmd {
        CoinCommand::Create { datastore, size, wallet } => {
            create_coin(ctx, &coin_manager, datastore, size, wallet)
        }
        CoinCommand::Mint { coin_id, wallet } => {
            mint_coin(&coin_manager, coin_id, wallet)
        }
        CoinCommand::List { active, owner, datastore } => {
            list_coins(&coin_manager, active, owner, datastore)
        }
        CoinCommand::Info { coin_id } => {
            show_coin_info(&coin_manager, coin_id)
        }
        CoinCommand::Transfer { coin_id, to, wallet } => {
            transfer_coin(&coin_manager, coin_id, to, wallet)
        }
        CoinCommand::Spend { coin_id, wallet } => {
            spend_coin(&coin_manager, coin_id, wallet)
        }
        CoinCommand::Stats => {
            show_stats(&coin_manager)
        }
        CoinCommand::Collateral { size } => {
            show_collateral_requirement(&coin_manager, size)
        }
    }
}

fn get_coin_manager() -> Result<DatastoreCoinManager> {
    let config_dir = directories::BaseDirs::new()
        .ok_or_else(|| crate::core::error::DigstoreError::internal("Could not determine config directory"))?
        .config_dir()
        .join("digstore")
        .join("coins");
    
    DatastoreCoinManager::new(config_dir)
}

fn create_coin(
    ctx: &CliContext,
    manager: &DatastoreCoinManager,
    datastore: String,
    size: Option<u64>,
    wallet_profile: Option<String>,
) -> Result<()> {
    // Load wallet
    let wallet_mgr = WalletManager::new_with_profile(wallet_profile)?;
    wallet_mgr.ensure_wallet_initialized()?;
    let wallet = wallet_mgr.get_wallet()?;
    
    // Determine datastore ID and size
    let (datastore_id, size_bytes) = if let Ok(store) = ctx.load_store() {
        // It's a local store path
        let root_hash = store.get_root_hash()?;
        let datastore_id = DatastoreId::from_hash(&root_hash);
        let size = size.unwrap_or_else(|| store.calculate_total_size());
        (datastore_id, size)
    } else {
        // It's a datastore ID
        let datastore_id = DatastoreId::new(datastore);
        let size = size.ok_or_else(|| {
            crate::core::error::DigstoreError::ValidationError {
                field: "size".to_string(),
                reason: "Size must be specified for external datastores".to_string(),
            }
        })?;
        (datastore_id, size)
    };
    
    // Get collateral requirement
    let collateral_req = manager.get_collateral_requirement(size_bytes)?;
    
    println!("{}", "Datastore Coin Creation".bold().cyan());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Datastore ID: {}", datastore_id.to_string().bright_white());
    println!("Size: {} bytes", size_bytes);
    println!();
    println!("{}", "Collateral Requirement:".yellow());
    println!("  Datastore size: {:.3} GB", collateral_req.breakdown.size_gb);
    println!("  Rate: {} DIG per GB", collateral_req.breakdown.rate_per_gb_dig);
    println!("  Base amount: {:.8} DIG", collateral_req.breakdown.base_calculation_dig);
    if collateral_req.breakdown.is_large_datastore {
        println!("  Large datastore multiplier: {}x", collateral_req.size_multiplier);
    }
    println!("  {} {:.8} DIG tokens", "Total required:".bold(), collateral_req.total_amount as f64 / 100_000_000.0);
    println!();
    
    // Create the coin
    let coin = manager.create_coin(
        datastore_id,
        ctx.load_store()?.get_root_hash()?,
        size_bytes,
        &wallet,
    )?;
    
    println!("{} Coin created successfully!", "✓".green());
    println!("Coin ID: {}", coin.id.to_string().bright_white());
    println!("State: {:?}", coin.state);
    println!();
    println!("Next step: Use 'digstore coin mint {}' to mint on blockchain", coin.id);
    
    Ok(())
}

fn mint_coin(
    manager: &DatastoreCoinManager,
    coin_id: String,
    wallet_profile: Option<String>,
) -> Result<()> {
    let coin_id = crate::datastore_coin::CoinId::new(coin_id);
    
    // Load wallet
    let wallet_mgr = WalletManager::new_with_profile(wallet_profile)?;
    wallet_mgr.ensure_wallet_initialized()?;
    let wallet = wallet_mgr.get_wallet()?;
    
    println!("Minting coin on blockchain...");
    let tx_id = manager.mint_coin(&coin_id, &wallet)?;
    
    println!("{} Coin minted successfully!", "✓".green());
    println!("Transaction ID: {}", tx_id.bright_white());
    
    Ok(())
}

fn list_coins(
    manager: &DatastoreCoinManager,
    active_only: bool,
    owner: Option<String>,
    datastore: Option<String>,
) -> Result<()> {
    let coins = if let Some(owner_addr) = owner {
        manager.get_coins_by_owner(&owner_addr)?
    } else if let Some(ds_id) = datastore {
        let datastore_id = DatastoreId::new(ds_id);
        manager.get_coins_by_datastore(&datastore_id)?
    } else {
        manager.list_coins(active_only)
    };
    
    if coins.is_empty() {
        println!("No coins found");
        return Ok(());
    }
    
    #[derive(Tabled)]
    struct CoinRow {
        #[tabled(rename = "Coin ID")]
        id: String,
        #[tabled(rename = "State")]
        state: String,
        #[tabled(rename = "Datastore")]
        datastore: String,
        #[tabled(rename = "Size")]
        size: String,
        #[tabled(rename = "Collateral")]
        collateral: String,
        #[tabled(rename = "Owner")]
        owner: String,
    }
    
    let rows: Vec<CoinRow> = coins.into_iter().map(|coin| {
        CoinRow {
            id: coin.id.to_string(),
            state: format!("{:?}", coin.state),
            datastore: coin.metadata.datastore_id.to_string(),
            size: bytesize::ByteSize(coin.metadata.size_bytes).to_string(),
            collateral: format!("{:.8} DIG", coin.metadata.collateral_amount as f64 / 100_000_000.0),
            owner: coin.metadata.owner_address.clone(),
        }
    }).collect();
    
    let table = Table::new(rows);
    println!("{}", table);
    
    Ok(())
}

fn show_coin_info(
    manager: &DatastoreCoinManager,
    coin_id: String,
) -> Result<()> {
    let coin_id = crate::datastore_coin::CoinId::new(coin_id);
    let coin = manager.get_coin(&coin_id)?;
    
    println!("{}", "Datastore Coin Information".bold().cyan());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Coin ID: {}", coin.id.to_string().bright_white());
    println!("State: {:?}", coin.state);
    println!();
    println!("{}", "Datastore:".yellow());
    println!("  ID: {}", coin.metadata.datastore_id);
    println!("  Root Hash: {}", coin.metadata.root_hash);
    println!("  Size: {} bytes", coin.metadata.size_bytes);
    println!();
    println!("{}", "Ownership:".yellow());
    println!("  Owner: {}", coin.metadata.owner_address);
    if let Some(host) = &coin.metadata.host_address {
        println!("  Host: {}", host);
    }
    println!();
    println!("{}", "Collateral:".yellow());
    println!("  Amount: {:.8} DIG tokens", coin.metadata.collateral_amount as f64 / 100_000_000.0);
    println!("  Created: {}", chrono::DateTime::<chrono::Utc>::from_timestamp(coin.metadata.created_at as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Unknown".to_string()));
    if let Some(expires) = coin.metadata.expires_at {
        println!("  Expires: {}", chrono::DateTime::<chrono::Utc>::from_timestamp(expires as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Unknown".to_string()));
    }
    println!();
    if let Some(tx_id) = &coin.tx_id {
        println!("{}", "Blockchain:".yellow());
        println!("  Transaction: {}", tx_id);
        if let Some(height) = coin.block_height {
            println!("  Block Height: {}", height);
        }
    }
    
    Ok(())
}

fn transfer_coin(
    manager: &DatastoreCoinManager,
    coin_id: String,
    to_address: String,
    wallet_profile: Option<String>,
) -> Result<()> {
    let coin_id = crate::datastore_coin::CoinId::new(coin_id);
    
    // Load wallet
    let wallet_mgr = WalletManager::new_with_profile(wallet_profile)?;
    wallet_mgr.ensure_wallet_initialized()?;
    let wallet = wallet_mgr.get_wallet()?;
    
    println!("Transferring coin ownership...");
    manager.transfer_coin(&coin_id, &wallet, &to_address)?;
    
    println!("{} Coin transferred successfully!", "✓".green());
    println!("New owner: {}", to_address.bright_white());
    
    Ok(())
}

fn spend_coin(
    manager: &DatastoreCoinManager,
    coin_id: String,
    wallet_profile: Option<String>,
) -> Result<()> {
    let coin_id = crate::datastore_coin::CoinId::new(coin_id);
    
    // Load wallet
    let wallet_mgr = WalletManager::new_with_profile(wallet_profile)?;
    wallet_mgr.ensure_wallet_initialized()?;
    let wallet = wallet_mgr.get_wallet()?;
    
    println!("Spending coin to release collateral...");
    let refund_amount = manager.spend_coin(&coin_id, &wallet)?;
    
    println!("{} Coin spent successfully!", "✓".green());
    println!("Collateral refunded: {:.8} DIG tokens", refund_amount as f64 / 100_000_000.0);
    
    Ok(())
}

fn show_stats(manager: &DatastoreCoinManager) -> Result<()> {
    let stats = manager.get_stats();
    
    println!("{}", "Datastore Coin Statistics".bold().cyan());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Total coins: {}", stats.total_coins);
    println!("Active coins: {}", stats.active_coins);
    println!("Pending coins: {}", stats.pending_coins);
    println!("Expired coins: {}", stats.expired_coins);
    println!("Spent coins: {}", stats.spent_coins);
    println!();
    println!("Total collateral locked: {:.8} DIG tokens", stats.total_collateral_locked as f64 / 100_000_000.0);
    println!("Total storage: {}", bytesize::ByteSize(stats.total_storage_bytes));
    
    Ok(())
}

fn show_collateral_requirement(
    manager: &DatastoreCoinManager,
    size_bytes: u64,
) -> Result<()> {
    let req = manager.get_collateral_requirement(size_bytes)?;
    
    println!("{}", "Collateral Requirement Calculator".bold().cyan());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Datastore size: {}", bytesize::ByteSize(size_bytes));
    println!();
    println!("Rate: {} DIG per GB", req.breakdown.rate_per_gb_dig);
    println!("Base amount: {:.8} DIG tokens", req.base_amount as f64 / 100_000_000.0);
    if req.breakdown.is_large_datastore {
        println!();
        println!("{}", "Large datastore detected!".yellow());
        println!("Multiplier: {}x", req.size_multiplier);
    }
    println!();
    println!("{} {:.8} DIG tokens", "Total required:".bold(), req.total_amount as f64 / 100_000_000.0);
    
    Ok(())
}