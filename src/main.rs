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

use cli::{Cli, Commands};

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
        Commands::Add { paths, recursive, all, force, dry_run, from_stdin } => {
            cli::commands::add::execute(paths, recursive, all, force, dry_run, from_stdin)
        }
        Commands::Commit { message, full_layer, author, date, edit } => {
            cli::commands::commit::execute(message, full_layer, author, date, edit)
        }
        Commands::Status { short, porcelain, show_chunks } => {
            cli::commands::status::execute(short, porcelain, show_chunks)
        }
        Commands::Get { path, output, verify, metadata, at, progress } => {
            cli::commands::get::execute(path, output, verify, metadata, at, progress)
        }
        Commands::Cat { path, at, number, no_pager, bytes } => {
            cli::commands::cat::execute(path, at, number, no_pager, bytes)
        }
        Commands::Prove { target, output, format, at, bytes, compact } => {
            cli::commands::prove::execute(target, output, format, at, bytes, compact)
        }
        Commands::Verify { proof, target, root, verbose, from_stdin } => {
            cli::commands::verify::execute(proof, target, root, verbose, from_stdin)
        }
        Commands::Log { limit, oneline, graph, since } => {
            cli::commands::log::execute(limit, oneline, graph, since)
        }
        Commands::Info { json, layer } => {
            cli::commands::info::execute(json, layer)
        }
        Commands::Completion { shell } => {
            cli::commands::completion::execute(shell)
        }
    }
}
