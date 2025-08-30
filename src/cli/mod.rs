//! Command-line interface for Digstore Min

use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod commands;

/// Digstore Min - Content-addressable storage system
#[derive(Parser)]
#[command(
    name = "digstore",
    version,
    about = "A simplified content-addressable storage system with Git-like semantics",
    long_about = "Digstore Min provides Git-like repository functionality with enhanced merkle proof capabilities and URN-based retrieval."
)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress non-error output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable progress bars
    #[arg(long, global = true)]
    pub no_progress: bool,

    /// Color output: auto, always, never
    #[arg(long, default_value = "auto", global = true)]
    pub color: String,

    /// Path to store directory
    #[arg(long, global = true)]
    pub store: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new repository
    Init {
        /// Use specific store ID (default: generate random)
        #[arg(long)]
        store_id: Option<String>,
        
        /// Repository name
        #[arg(long)]
        name: Option<String>,
        
        /// Disable compression
        #[arg(long)]
        no_compression: bool,
        
        /// Average chunk size in KB (default: 1024)
        #[arg(long, default_value = "1024")]
        chunk_size: u32,
    },

    /// Add files to the repository
    Add {
        /// Files or directories to add
        paths: Vec<PathBuf>,
        
        /// Add directories recursively
        #[arg(short, long)]
        recursive: bool,
        
        /// Add all files in the repository
        #[arg(short = 'A', long)]
        all: bool,
        
        /// Force add ignored files
        #[arg(short, long)]
        force: bool,
        
        /// Show what would be added
        #[arg(long)]
        dry_run: bool,
        
        /// Read file list from stdin
        #[arg(long)]
        from_stdin: bool,
    },

    /// Create a new commit
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
        
        /// Create full layer (not delta)
        #[arg(long)]
        full_layer: bool,
        
        /// Set author name
        #[arg(long)]
        author: Option<String>,
        
        /// Override commit date
        #[arg(long)]
        date: Option<String>,
        
        /// Open editor for message
        #[arg(short, long)]
        edit: bool,
    },

    /// Show repository status
    Status {
        /// Show short format
        #[arg(short, long)]
        short: bool,
        
        /// Machine-readable output
        #[arg(long)]
        porcelain: bool,
        
        /// Display chunk statistics
        #[arg(long)]
        show_chunks: bool,
    },

    /// Retrieve files from the repository
    Get {
        /// Path or URN to retrieve
        path: String,
        
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        
        /// Verify with merkle proof while retrieving
        #[arg(long)]
        verify: bool,
        
        /// Include metadata in output
        #[arg(long)]
        metadata: bool,
        
        /// Retrieve at specific root hash
        #[arg(long)]
        at: Option<String>,
        
        /// Force show progress even when piping
        #[arg(long)]
        progress: bool,
    },

    /// Display file contents from repository
    Cat {
        /// File path or URN
        path: String,
        
        /// Show at specific root hash
        #[arg(long)]
        at: Option<String>,
        
        /// Number all output lines
        #[arg(short, long)]
        number: bool,
        
        /// Don't use pager for long output
        #[arg(long)]
        no_pager: bool,
        
        /// Display specific byte range
        #[arg(long)]
        bytes: Option<String>,
    },

    /// Generate merkle proof for content
    Prove {
        /// Target to prove
        target: String,
        
        /// Write proof to file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        
        /// Output format: json, binary, text
        #[arg(long, default_value = "json")]
        format: String,
        
        /// Prove at specific root hash
        #[arg(long)]
        at: Option<String>,
        
        /// Prove specific byte range
        #[arg(long)]
        bytes: Option<String>,
        
        /// Generate compact proof
        #[arg(long)]
        compact: bool,
    },

    /// Verify a merkle proof
    Verify {
        /// Proof file to verify
        proof: PathBuf,
        
        /// Expected target hash
        #[arg(long)]
        target: Option<String>,
        
        /// Expected root hash
        #[arg(long)]
        root: Option<String>,
        
        /// Show detailed verification steps
        #[arg(short, long)]
        verbose: bool,
        
        /// Read proof from stdin
        #[arg(long)]
        from_stdin: bool,
    },

    /// Show commit history
    Log {
        /// Limit number of entries
        #[arg(short = 'n', long)]
        limit: Option<usize>,
        
        /// One line per layer
        #[arg(long)]
        oneline: bool,
        
        /// Show ASCII graph
        #[arg(long)]
        graph: bool,
        
        /// Show layers since date
        #[arg(long)]
        since: Option<String>,
    },

    /// Display repository information
    Info {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        
        /// Show specific layer info
        #[arg(long)]
        layer: Option<String>,
    },
}
