//! Command-line interface for Digstore Min

use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod commands;
pub mod interactive;

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

    /// Auto-answer yes to all prompts
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

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

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Decrypt encrypted content using URN
    Decrypt {
        /// Input file containing encrypted data
        path: String,

        /// Output file for decrypted data (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// URN to use for decryption (if not auto-detected)
        #[arg(long)]
        urn: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate shell completion scripts
    Completion {
        /// Shell to generate completion for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Store management and information
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },

    /// Proof system
    Proof {
        #[command(subcommand)]
        command: ProofCommands,
    },

    /// Layer management
    Layer {
        #[command(subcommand)]
        command: LayerCommands,
    },

    /// Staging area management
    Staged {
        #[command(subcommand)]
        command: StagedCommands,
    },

    /// Manage global configuration
    Config {
        /// Configuration key to get/set
        key: Option<String>,
        /// Configuration value to set
        value: Option<String>,
        /// List all configuration values
        #[arg(short, long)]
        list: bool,
        /// Unset a configuration value
        #[arg(long)]
        unset: bool,
        /// Show global configuration file location
        #[arg(long)]
        show_origin: bool,
        /// Edit configuration file in editor
        #[arg(short, long)]
        edit: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Staging area subcommands
#[derive(Subcommand)]
pub enum StagedCommands {
    /// List staged files
    List {
        /// Number of files to show per page
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Page number (1-based)
        #[arg(short, long, default_value = "1")]
        page: usize,
        /// Show detailed information (file sizes, hashes)
        #[arg(short, long)]
        detailed: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Show all files (no pagination)
        #[arg(short, long)]
        all: bool,
    },

    /// Show differences between staged files and last commit
    Diff {
        /// Show only file names (no content diff)
        #[arg(long)]
        name_only: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Show statistics summary
        #[arg(long)]
        stat: bool,
        /// Number of context lines for content diff
        #[arg(short = 'U', long, default_value = "3")]
        unified: usize,
        /// Specific file to diff (default: all staged files)
        file: Option<String>,
    },

    /// Clear all staged files
    Clear {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Force clear without confirmation
        #[arg(short, long)]
        force: bool,
    },
}

/// Layer management subcommands
#[derive(Subcommand)]
pub enum LayerCommands {
    /// List all layers
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show size information
        #[arg(long)]
        size: bool,

        /// Show file details
        #[arg(long)]
        files: bool,

        /// Show chunk details
        #[arg(long)]
        chunks: bool,
    },

    /// Analyze specific layer
    Analyze {
        /// Layer hash to analyze
        layer_hash: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show size information
        #[arg(long)]
        size: bool,

        /// Show file details
        #[arg(long)]
        files: bool,

        /// Show chunk details
        #[arg(long)]
        chunks: bool,
    },

    /// Deep layer inspection
    Inspect {
        /// Layer hash to inspect
        layer_hash: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show layer header details
        #[arg(long)]
        header: bool,

        /// Show merkle tree information
        #[arg(long)]
        merkle: bool,

        /// Show chunk analysis
        #[arg(long)]
        chunks: bool,

        /// Verify layer integrity
        #[arg(long)]
        verify: bool,
    },
}

/// Store management subcommands
#[derive(Subcommand)]
pub enum StoreCommands {
    /// Show store information
    Info {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show configuration details
        #[arg(long)]
        config: bool,

        /// Show all paths
        #[arg(long)]
        paths: bool,

        /// Show specific layer info
        #[arg(long)]
        layer: Option<String>,
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

    /// Show root history analysis
    History {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Limit number of entries
        #[arg(short = 'n', long)]
        limit: Option<usize>,

        /// Show statistics
        #[arg(long)]
        stats: bool,

        /// Show ASCII graph
        #[arg(long)]
        graph: bool,

        /// Show entries since date
        #[arg(long)]
        since: Option<String>,
    },

    /// Show current root information
    Root {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,

        /// Show only the root hash
        #[arg(long)]
        hash_only: bool,
    },

    /// Show storage analytics
    Size {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show detailed breakdown
        #[arg(long)]
        breakdown: bool,

        /// Show deduplication metrics
        #[arg(long)]
        efficiency: bool,

        /// Show per-layer analysis
        #[arg(long)]
        layers: bool,
    },

    /// Show store statistics
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show detailed statistics
        #[arg(long)]
        detailed: bool,

        /// Show performance metrics
        #[arg(long)]
        performance: bool,

        /// Show security metrics
        #[arg(long)]
        security: bool,
    },
}

/// Proof system subcommands
#[derive(Subcommand)]
pub enum ProofCommands {
    /// Generate merkle proof for content
    Generate {
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
}
