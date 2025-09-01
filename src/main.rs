//! Digstore Min CLI
//!
//! Command-line interface for the Digstore Min content-addressable storage system.

use clap::Parser;
use anyhow::Result;
use tracing_subscriber;

mod core;
mod storage;
mod proofs;
mod urn;
mod cli;
mod security;
mod ignore;
mod config;

use cli::{Cli, Commands, StagedCommands, LayerCommands, StoreCommands, ProofCommands};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Parse command line arguments
    let cli = Cli::parse();
    
    // Execute the command
    match cli.command {
        Commands::Init { store_id, name, no_compression, chunk_size } => {
            cli::commands::init::execute(store_id, name, no_compression, chunk_size)
        }
        Commands::Add { paths, recursive, all, force, dry_run, from_stdin, json } => {
            cli::commands::add::execute(paths, recursive, all, force, dry_run, from_stdin, cli.yes, json)
        }
        Commands::Commit { message, full_layer, author, date, edit, json } => {
            cli::commands::commit::execute(message, full_layer, author, date, edit, json)
        }
        Commands::Status { short, porcelain, show_chunks, json } => {
            cli::commands::status::execute(short, porcelain, show_chunks, json)
        }
        Commands::Get { path, output, verify, metadata, at, progress } => {
            cli::commands::get::execute(path, output, verify, metadata, at, progress)
        }
        Commands::Cat { path, at, number, no_pager, bytes } => {
            cli::commands::cat::execute(path, at, number, no_pager, bytes)
        }
        Commands::Completion { shell } => {
            cli::commands::completion::execute(shell)
        }

        Commands::Staged { command } => {
            match command {
                StagedCommands::List { limit, page, detailed, json, all } => {
                    cli::commands::staged::execute_list(limit, page, detailed, json, all)
                }
                StagedCommands::Diff { name_only, json, stat, unified, file } => {
                    cli::commands::staged::execute_diff(name_only, json, stat, unified, file)
                }
                StagedCommands::Clear { json, force } => {
                    cli::commands::staged::clear_staged(json, force)
                }
            }
        }

        Commands::Layer { command } => {
            match command {
                LayerCommands::List { json, size, files, chunks } => {
                    cli::commands::layer::execute_list(None, json, true, size, files, chunks)
                }
                LayerCommands::Analyze { layer_hash, json, size, files, chunks } => {
                    cli::commands::layer::execute_list(Some(layer_hash), json, false, size, files, chunks)
                }
                LayerCommands::Inspect { layer_hash, json, header, merkle, chunks, verify } => {
                    cli::commands::layer::execute_inspect(layer_hash, json, header, merkle, chunks, verify)
                }
            }
        }

        Commands::Store { command } => {
            match command {
                StoreCommands::Info { json, config, paths, layer } => {
                    if layer.is_some() {
                        cli::commands::store::execute_info(json, layer)
                    } else {
                        cli::commands::store::execute_store_info(json, config, paths)
                    }
                }
                StoreCommands::Log { limit, oneline, graph, since } => {
                    cli::commands::store::execute_log(limit, oneline, graph, since)
                }
                StoreCommands::History { json, limit, stats, graph, since } => {
                    cli::commands::store::execute_history(json, limit, stats, graph, since)
                }
                StoreCommands::Root { json, verbose, hash_only } => {
                    cli::commands::store::execute_root(json, verbose, hash_only)
                }
                StoreCommands::Size { json, breakdown, efficiency, layers } => {
                    cli::commands::store::execute_size(json, breakdown, efficiency, layers)
                }
                StoreCommands::Stats { json, detailed, performance, security } => {
                    cli::commands::store::execute_stats(json, detailed, performance, security)
                }
            }
        }

        Commands::Proof { command } => {
            match command {
                ProofCommands::Generate { target, output, format, at, bytes, compact } => {
                    cli::commands::proof::execute_generate(target, output, format, at, bytes, compact)
                }
                ProofCommands::Verify { proof, target, root, verbose, from_stdin } => {
                    cli::commands::proof::execute_verify(proof, target, root, verbose, from_stdin)
                }
            }
        }

        Commands::Config { key, value, list, unset, show_origin, edit, json } => {
            cli::commands::config::execute(key, value, list, unset, show_origin, edit, json)
        }
    }
}
