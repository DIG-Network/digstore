//! `clap` command-line surface for the `digstore` binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "digstore", version, about, long_about = None)]
pub struct Cli {
    #[arg(long, global = true)]
    pub dig_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// Color output: auto (default), always, or never.
    #[arg(long, global = true, default_value = "auto")]
    pub color: crate::ui::ColorChoice,
    /// Suppress progress and hints.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Init(InitArgs),
    Add(AddArgs),
    Commit(CommitArgs),
    Status(StatusArgs),
    Log(LogArgs),
    Diff(DiffArgs),
    Checkout(CheckoutArgs),
    Cat(CatArgs),
    Remote(RemoteArgs),
    Clone(CloneArgs),
    Push(PushArgs),
    Pull(PullArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long)]
    pub private: bool,
    #[arg(long)]
    pub data_dir: Option<String>,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Path to the file to stage. Omitted (or ignored) when `--discovery` is set.
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub key: Option<String>,
    /// §8.5 social conventions: instead of staging a file, generate and stage the
    /// `/.well-known/dig/manifest.json` discovery manifest listing the resources
    /// already staged (publisher-elected keys, labels, and types).
    #[arg(long)]
    pub discovery: bool,
}

#[derive(Debug, Args)]
pub struct CommitArgs {
    #[arg(short, long)]
    pub message: Option<String>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {}

#[derive(Debug, Args)]
pub struct LogArgs {
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Args)]
pub struct CheckoutArgs {
    pub root: String,
    #[arg(long, short)]
    pub out: PathBuf,
    #[arg(long)]
    pub salt: Option<String>,
}

#[derive(Debug, Args)]
pub struct CatArgs {
    pub urn: String,
    #[arg(long)]
    pub salt: Option<String>,
    #[arg(long)]
    pub verify_proof: bool,
}

#[derive(Debug, Args)]
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
pub struct CloneArgs {
    pub source: String,
}

#[derive(Debug, Args)]
pub struct PushArgs {
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    #[arg(default_value = "origin")]
    pub remote: String,
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
            Command::Add(a) => assert_eq!(a.path.unwrap().to_str().unwrap(), "file.txt"),
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
