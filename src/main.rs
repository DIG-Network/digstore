//! DigStore CLI
//!
//! Command-line interface for the DigStore content-addressable storage system.

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_strip)]
#![allow(clippy::type_complexity)]
#![allow(clippy::inherent_to_string)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::manual_clamp)]
#![allow(deprecated)]
#![allow(clippy::wrong_self_convention)]

use anyhow::Result;
use clap::Parser;

mod cli;
mod config;
mod core;
mod crypto;
mod ignore;
mod proofs;
mod security;
mod storage;
mod update;
mod urn;
mod wallet;

use cli::{
    context::CliContext, Cli, Commands, LayerCommands, ProofCommands, StagedCommands, StoreCommands,
};
use wallet::WalletManager;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Parse command line arguments
    let cli = Cli::parse();

    // Extract custom keys from the command arguments
    let (custom_encryption_key, custom_decryption_key) = match &cli.command {
        Commands::Decrypt { decryption_key, .. } => (None, decryption_key.clone()),
        _ => (None, None),
    };

    // Set CLI context for commands to access
    CliContext::set(CliContext {
        wallet_profile: cli.wallet_profile.clone(),
        auto_generate_wallet: cli.auto_generate_wallet,
        auto_import_mnemonic: cli.auto_import_mnemonic.clone(),
        verbose: cli.verbose,
        quiet: cli.quiet,
        yes: cli.yes,
        non_interactive: cli.non_interactive,
        custom_encryption_key,
        custom_decryption_key,
    });

    // Handle auto-generation or auto-import flags first (these work regardless of command)
    if cli.auto_generate_wallet || cli.auto_import_mnemonic.is_some() {
        let wallet_profile = cli.wallet_profile.clone();
        let wallet_manager = WalletManager::new_with_profile(wallet_profile)?;

        if cli.auto_generate_wallet {
            wallet_manager.auto_generate_wallet()?;
        } else if let Some(mnemonic) = cli.auto_import_mnemonic.clone() {
            wallet_manager.auto_import_wallet(&mnemonic)?;
        }
    }

    // Check if wallet initialization is needed for this command
    if needs_wallet_initialization(&cli.command) {
        let wallet_profile = cli.wallet_profile.clone();
        let wallet_manager = WalletManager::new_with_profile(wallet_profile)?;
        wallet_manager.ensure_wallet_initialized()?;
    }

    // Check for updates (unless running update command, in quiet mode, or non-interactive)
    if !matches!(cli.command, Commands::Update { .. }) && !cli.quiet && !cli.non_interactive {
        check_and_prompt_for_updates()?;
    }

    // Execute the command
    match cli.command {
        Commands::Init {
            name,
            encryption_key,
        } => cli::commands::init::execute(name, encryption_key),
        Commands::Add {
            paths,
            recursive,
            all,
            force,
            dry_run,
            from_stdin,
            json,
        } => cli::commands::add::execute(
            paths, recursive, all, force, dry_run, from_stdin, cli.yes, json,
        ),
        Commands::Commit {
            message,
            author,
            date,
            edit,
            json,
        } => cli::commands::commit::execute(message, author, date, edit, json),
        Commands::Status {
            short,
            porcelain,
            show_chunks,
            json,
        } => cli::commands::status::execute(short, porcelain, show_chunks, json),
        Commands::Get {
            path,
            output,
            verify,
            metadata,
            at,
            progress,
            json,
        } => cli::commands::get::execute(path, output, verify, metadata, at, progress, json),
        Commands::Decrypt {
            path,
            output,
            urn,
            decryption_key,
            json,
        } => cli::commands::decrypt::execute(path, output, urn, decryption_key, json),
        Commands::Keygen {
            urn,
            output,
            storage_address,
            encryption_key,
            json,
        } => cli::commands::keygen::execute(urn, output, storage_address, encryption_key, json),
        Commands::Completion { shell } => cli::commands::completion::execute(shell),

        Commands::Staged { command } => match command {
            StagedCommands::List {
                limit,
                page,
                detailed,
                json,
                all,
            } => cli::commands::staged::execute_list(limit, page, detailed, json, all),
            StagedCommands::Diff {
                name_only,
                json,
                stat,
                unified,
                file,
            } => cli::commands::staged::execute_diff(name_only, json, stat, unified, file),
            StagedCommands::Clear { json, force } => {
                cli::commands::staged::clear_staged(json, force)
            },
        },

        Commands::Update {
            check_only,
            force,
            json,
        } => cli::commands::update::execute(check_only, force, json),

        Commands::Version {
            subcommand,
            version,
        } => cli::commands::version_cmd::execute(subcommand, version)
            .map_err(|e| anyhow::anyhow!("Version command failed: {}", e)),

        Commands::Layer { command } => match command {
            LayerCommands::List {
                json,
                size,
                files,
                chunks,
            } => cli::commands::layer::execute_list(None, json, true, size, files, chunks),
            LayerCommands::Analyze {
                layer_hash,
                json,
                size,
                files,
                chunks,
            } => cli::commands::layer::execute_list(
                Some(layer_hash),
                json,
                false,
                size,
                files,
                chunks,
            ),
            LayerCommands::Inspect {
                layer_hash,
                json,
                header,
                merkle,
                chunks,
                verify,
            } => cli::commands::layer::execute_inspect(
                layer_hash, json, header, merkle, chunks, verify,
            ),
        },

        Commands::Store { command } => match command {
            StoreCommands::Info {
                json,
                config,
                paths,
                layer,
            } => {
                if layer.is_some() {
                    cli::commands::store::execute_info(json, layer)
                } else {
                    cli::commands::store::execute_store_info(json, config, paths)
                }
            },
            StoreCommands::Log {
                limit,
                oneline,
                graph,
                since,
            } => cli::commands::store::execute_log(limit, oneline, graph, since),
            StoreCommands::History {
                json,
                limit,
                stats,
                graph,
                since,
            } => cli::commands::store::execute_history(json, limit, stats, graph, since),
            StoreCommands::Root {
                json,
                verbose,
                hash_only,
            } => cli::commands::store::execute_root(json, verbose, hash_only),
            StoreCommands::Size {
                json,
                breakdown,
                efficiency,
                layers,
            } => cli::commands::store::execute_size(json, breakdown, efficiency, layers),
            StoreCommands::Stats {
                json,
                detailed,
                performance,
                security,
            } => cli::commands::store::execute_stats(json, detailed, performance, security),
        },

        Commands::Proof { command } => match command {
            ProofCommands::Generate {
                target,
                output,
                format,
                at,
                bytes,
            } => cli::commands::proof::execute_generate(target, output, format, at, bytes),
            ProofCommands::Verify {
                proof,
                target,
                root,
                verbose,
                from_stdin,
            } => cli::commands::proof::execute_verify(proof, target, root, verbose, from_stdin),
            ProofCommands::GenerateArchiveSize {
                store_id,
                output,
                format,
                verbose,
                show_compression,
                json,
            } => cli::commands::proof::execute_generate_archive_size(
                store_id,
                output,
                format,
                verbose,
                show_compression,
                json,
            ),
            ProofCommands::VerifyArchiveSize {
                proof,
                store_id,
                root_hash,
                expected_size,
                publisher_public_key,
                from_file,
                verbose,
                json,
            } => cli::commands::proof::execute_verify_archive_size(
                proof,
                store_id,
                root_hash,
                expected_size,
                publisher_public_key,
                from_file,
                verbose,
                json,
            ),
        },

        Commands::Config {
            key,
            value,
            list,
            unset,
            show_origin,
            edit,
            json,
        } => cli::commands::config::execute(key, value, list, unset, show_origin, edit, json),

        Commands::Wallet { command } => {
            cli::commands::wallet::execute(command).map_err(|e| e.into())
        },
    }
}

/// Determines if a command requires wallet initialization
fn needs_wallet_initialization(command: &Commands) -> bool {
    match command {
        // Commands that don't need wallet initialization
        Commands::Completion { .. } => false,
        Commands::Config { .. } => false,
        Commands::Update { .. } => false,
        Commands::Version { .. } => false,
        Commands::Wallet { .. } => false, // Wallet commands manage their own initialization

        // All other commands require wallet initialization
        _ => true,
    }
}

/// Check for updates and prompt user if available
fn check_and_prompt_for_updates() -> Result<()> {
    use colored::Colorize;
    use dialoguer::Confirm;

    // Only check occasionally to avoid slowing down commands
    let update_info = match crate::update::check_for_updates() {
        Ok(info) => info,
        Err(_) => return Ok(()), // Silently ignore update check failures
    };

    if !update_info.update_available {
        return Ok(());
    }

    println!();
    println!(
        "{} {} Update available: {} → {}",
        "🎉".bright_yellow(),
        "Digstore".bright_cyan().bold(),
        update_info.current_version.dimmed(),
        update_info.latest_version.bright_green()
    );

    let should_update = if crate::cli::context::CliContext::is_non_interactive() {
        false // Don't auto-update in non-interactive mode
    } else {
        Confirm::new()
            .with_prompt("Would you like to download and install the update now?")
            .default(false)
            .interact()
            .unwrap_or(false)
    };

    if should_update {
        println!();
        if let Some(download_url) = update_info.download_url {
            match crate::update::download_and_install_update(&download_url) {
                Ok(_) => {
                    println!(
                        "{}",
                        "✓ Update completed! Please restart your terminal."
                            .green()
                            .bold()
                    );
                },
                Err(e) => {
                    println!("{} Update failed: {}", "✗".red(), e);
                    println!(
                        "  {} Manual download: https://github.com/DIG-Network/digstore/releases",
                        "→".cyan()
                    );
                },
            }
        }
    } else {
        println!("  {} Use 'digstore update' to update later", "→".cyan());
    }

    println!();
    Ok(())
}
