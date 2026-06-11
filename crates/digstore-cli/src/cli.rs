//! `clap` command-line surface for the `digstore` binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "digstore", version, about, long_about = None)]
pub struct Cli {
    /// Override the .dig metadata directory (default: the workspace's .dig).
    #[arg(long, global = true)]
    pub dig_dir: Option<PathBuf>,
    /// Emit machine-readable JSON instead of human-formatted output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Enable verbose (debug-level) logging.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// Color output: auto (default), always, or never.
    #[arg(long, global = true, default_value = "auto")]
    pub color: crate::ui::ColorChoice,
    /// Suppress progress and hints.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Operate on a specific store by name (overrides the active store).
    #[arg(long = "store", global = true)]
    pub store_name: Option<String>,
    /// Operating directory for add/urn/status (overrides the store's content root).
    #[arg(short = 'C', long = "cwd", global = true)]
    pub cwd: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a new store in the current directory.
    Init(InitArgs),
    /// Stage files, directories, or glob patterns for the next commit.
    Add(AddArgs),
    /// Commit the staged content as a new generation root.
    Commit(CommitArgs),
    /// Show the active store, its content root, and pending staged changes.
    Status(StatusArgs),
    /// Show the store's generation (commit) history.
    Log(LogArgs),
    /// Show what changed between two generation roots.
    Diff(DiffArgs),
    /// Materialize a generation root's content into an output directory.
    Checkout(CheckoutArgs),
    /// Stream a resource out by URN (decrypted) or retrieval key (encrypted).
    Cat(CatArgs),
    /// Manage remote endpoints for this store (add, list, remove).
    Remote(RemoteArgs),
    /// Clone a store from a remote into the current directory.
    Clone(CloneArgs),
    /// Push the local store's content and signed head to a remote.
    Push(PushArgs),
    /// Pull the latest content and signed head from a remote.
    Pull(PullArgs),
    /// Revoke a published root (or the whole store) with a signed tombstone.
    Revoke(RevokeArgs),
    /// List the stores in this workspace.
    Stores(StoresArgs),
    /// Switch the active store by name.
    Use(UseArgs),
    /// Show or set the active store's content root directory.
    Dir(DirArgs),
    /// Clear the staging area (discard all staged entries).
    Unstage(UnstageArgs),
    /// List the files currently staged for the next commit.
    Staged(StagedArgs),
    /// Print the URN(s) for staged or committed resources.
    Urn(UrnArgs),
    /// List the retrieval key (and URN) for every committed resource.
    Keys(KeysArgs),
    /// Update DigStore to the latest release.
    Update(UpdateArgs),
    /// Manage the encrypted wallet seed in ~/.dig.
    Seed(SeedArgs),
    /// Lock the seed (clear the cached-unlock session).
    Lock(LockArgs),
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore init\n  digstore init site --dir dist\n  digstore init --private"
)]
pub struct InitArgs {
    /// Store name (default: "default").
    pub name: Option<String>,
    #[arg(long)]
    pub private: bool,
    /// Content root (the build-output directory this store captures).
    #[arg(long)]
    pub dir: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore add file.txt\n  digstore add -A\n  digstore add . src/*.rs\n  digstore add logo.png --key assets/logo.png"
)]
pub struct AddArgs {
    /// Files, directories, or glob patterns to stage (relative to the store root).
    pub paths: Vec<PathBuf>,
    /// Stage every file under the store root (honoring .digignore/.gitignore).
    #[arg(short = 'A', long)]
    pub all: bool,
    /// Show what would be staged without staging anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Resource key override (only valid with exactly one file path).
    #[arg(long)]
    pub key: Option<String>,
    /// Stage the /.well-known/dig/manifest.json discovery manifest.
    #[arg(long)]
    pub discovery: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore commit -m \"first generation\"")]
pub struct CommitArgs {
    #[arg(short, long)]
    pub message: Option<String>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore status")]
pub struct StatusArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore log\n  digstore log --limit 10")]
pub struct LogArgs {
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore diff <rootA> <rootB>")]
pub struct DiffArgs {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore checkout <root> --out ./out")]
pub struct CheckoutArgs {
    pub root: String,
    #[arg(long, short)]
    pub out: PathBuf,
    #[arg(long)]
    pub salt: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore cat urn:dig:chia:<storeID>:<root>/readme\n  digstore cat urn:dig:chia:<storeID>/logo.png --out logo.png\n  digstore cat <64-hex-retrieval-key> --out blob.enc"
)]
pub struct CatArgs {
    /// A `urn:dig:…` (streamed out DECRYPTED) or a 64-char hex retrieval key
    /// (streamed out as RAW ENCRYPTED bytes, resolved within the active store).
    pub urn: String,
    /// Write output to this file instead of stdout.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Decryption salt (32-byte hex) for a private store.
    #[arg(long)]
    pub salt: Option<String>,
    /// Verify the resource's merkle proof against the trusted root before output.
    #[arg(long)]
    pub verify_proof: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore remote add origin https://host/stores/<storeID>")]
pub struct RemoteArgs {
    #[command(subcommand)]
    pub action: RemoteAction,
}

#[derive(Debug, Subcommand)]
pub enum RemoteAction {
    Add { name: String, url: String },
    List,
    Remove { name: String },
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore clone https://host/stores/<storeID>")]
pub struct CloneArgs {
    pub source: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore push origin")]
pub struct PushArgs {
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore pull origin")]
pub struct PullArgs {
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore revoke --root <hex> --reason compromise\n  digstore revoke --all --reason takedown\n  digstore revoke --root <hex> --remote origin"
)]
pub struct RevokeArgs {
    /// Revoke a single generation root (hex). Mutually exclusive with `--all`.
    #[arg(long, conflicts_with = "all")]
    pub root: Option<String>,
    /// Revoke the whole store (Store-scoped tombstone). Mutually exclusive with `--root`.
    #[arg(long)]
    pub all: bool,
    /// Why the root/store is revoked: unspecified (default), compromise, superseded, takedown.
    #[arg(long, default_value = "unspecified")]
    pub reason: String,
    /// The configured remote to publish the tombstone to.
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore stores")]
pub struct StoresArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore use site")]
pub struct UseArgs {
    pub name: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore dir\n  digstore dir dist")]
pub struct DirArgs {
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore unstage")]
pub struct UnstageArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore staged")]
pub struct StagedArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore urn -A\n  digstore urn css/app.css\n  digstore urn file --root <hex>"
)]
pub struct UrnArgs {
    pub paths: Vec<PathBuf>,
    #[arg(short = 'A', long)]
    pub all: bool,
    #[arg(long)]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore keys\n  digstore keys --root <hex>\n  digstore keys --json")]
pub struct KeysArgs {
    /// Generation root to list (hex); defaults to the current root.
    #[arg(long)]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
pub struct SeedArgs {
    #[command(subcommand)]
    pub action: SeedAction,
}

#[derive(Debug, Subcommand)]
pub enum SeedAction {
    /// Import an existing BIP-39 mnemonic.
    Import {
        /// Provide the mnemonic non-interactively (otherwise prompted).
        #[arg(long)]
        mnemonic: Option<String>,
    },
    /// Generate a new BIP-39 mnemonic.
    Generate {
        /// Word count (12/15/18/21/24).
        #[arg(long, default_value_t = 24, value_parser = parse_word_count)]
        words: usize,
    },
    /// Show whether a seed exists and is currently unlocked.
    Status,
}

fn parse_word_count(s: &str) -> Result<usize, String> {
    let n: usize = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
    match n {
        12 | 15 | 18 | 21 | 24 => Ok(n),
        _ => Err("word count must be one of 12, 15, 18, 21, 24".to_string()),
    }
}

#[derive(Debug, Args)]
pub struct LockArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore update\n  digstore update --check\n  digstore update --yes"
)]
pub struct UpdateArgs {
    /// Only report whether an update is available; never download.
    #[arg(long)]
    pub check: bool,
    /// Skip the confirmation prompt before downloading/running the installer.
    #[arg(short, long)]
    pub yes: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_init() {
        let cli = Cli::try_parse_from(["digstore", "init"]).unwrap();
        assert!(matches!(cli.command, Command::Init(_)));
    }

    #[test]
    fn parses_add_path() {
        let cli = Cli::try_parse_from(["digstore", "add", "file.txt"]).unwrap();
        match cli.command {
            Command::Add(a) => assert_eq!(a.paths[0].to_str().unwrap(), "file.txt"),
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn parses_cat_urn() {
        let cli = Cli::try_parse_from(["digstore", "cat", "urn:dig:chia:abcd/readme"]).unwrap();
        match cli.command {
            Command::Cat(c) => assert_eq!(c.urn, "urn:dig:chia:abcd/readme"),
            _ => panic!("expected cat"),
        }
    }

    #[test]
    fn parses_remote_add_subcommand() {
        let cli =
            Cli::try_parse_from(["digstore", "remote", "add", "origin", "https://h/stores/x"])
                .unwrap();
        match cli.command {
            Command::Remote(r) => match r.action {
                RemoteAction::Add { name, url } => {
                    assert_eq!(name, "origin");
                    assert_eq!(url, "https://h/stores/x");
                }
                _ => panic!("expected remote add"),
            },
            _ => panic!("expected remote"),
        }
    }

    #[test]
    fn parses_update_check_flag() {
        let cli = Cli::try_parse_from(["digstore", "update", "--check"]).unwrap();
        match cli.command {
            Command::Update(u) => {
                assert!(u.check);
                assert!(!u.yes);
            }
            _ => panic!("expected update"),
        }
    }

    #[test]
    fn parses_revoke_root_with_reason() {
        let cli = Cli::try_parse_from([
            "digstore", "revoke", "--root", "abcd", "--reason", "compromise",
        ])
        .unwrap();
        match cli.command {
            Command::Revoke(r) => {
                assert_eq!(r.root.as_deref(), Some("abcd"));
                assert!(!r.all);
                assert_eq!(r.reason, "compromise");
                assert_eq!(r.remote, "origin");
            }
            _ => panic!("expected revoke"),
        }
    }

    #[test]
    fn parses_revoke_all() {
        let cli = Cli::try_parse_from(["digstore", "revoke", "--all"]).unwrap();
        match cli.command {
            Command::Revoke(r) => {
                assert!(r.all);
                assert!(r.root.is_none());
            }
            _ => panic!("expected revoke"),
        }
    }

    #[test]
    fn revoke_rejects_root_and_all_together() {
        let err = Cli::try_parse_from(["digstore", "revoke", "--root", "ab", "--all"]);
        assert!(err.is_err(), "--root and --all are mutually exclusive");
    }

    #[test]
    fn parses_update_yes_flag() {
        let cli = Cli::try_parse_from(["digstore", "update", "--yes"]).unwrap();
        match cli.command {
            Command::Update(u) => assert!(u.yes),
            _ => panic!("expected update"),
        }
    }

    #[test]
    fn global_dig_dir_flag_before_subcommand() {
        let cli = Cli::try_parse_from(["digstore", "--dig-dir", "/tmp/d", "status"]).unwrap();
        assert_eq!(cli.dig_dir.unwrap().to_str().unwrap(), "/tmp/d");
    }

    #[test]
    fn global_json_flag_after_subcommand() {
        let cli = Cli::try_parse_from(["digstore", "status", "--json"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn private_salt_flag_on_cat() {
        let cli = Cli::try_parse_from([
            "digstore",
            "cat",
            "urn:dig:chia:abcd/r",
            "--salt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .unwrap();
        match cli.command {
            Command::Cat(c) => assert!(c.salt.is_some()),
            _ => panic!("expected cat"),
        }
    }
}
