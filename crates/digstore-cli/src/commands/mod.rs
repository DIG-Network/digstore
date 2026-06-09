//! Command dispatch: clap `Command` -> `ops` -> `output`.

use crate::cli::{Cli, Command};
use crate::context::CliContext;
use crate::error::CliError;

pub mod add;
pub mod cat;
pub mod checkout;
pub mod clone;
pub mod commit;
pub mod diff;
pub mod init;
pub mod log;
pub mod pull;
pub mod push;
pub mod remote;
pub mod status;

pub fn dispatch(cli: Cli) -> Result<(), CliError> {
    // Build the Ui once from the global flags; pass it by reference into every command.
    let ui = crate::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.verbose);
    // `init` anchors to the current directory; every other command discovers the
    // store by walking up from the current directory (Git-style), so the CLI runs
    // against the working directory it was invoked in.
    let explicit = cli.dig_dir.clone();
    let ctx = if matches!(cli.command, Command::Init(_)) {
        CliContext::resolve_init(explicit, cli.json, cli.verbose)
    } else {
        CliContext::resolve(explicit, cli.json, cli.verbose)
    };
    match cli.command {
        Command::Init(a) => init::run(&ctx, &ui, a),
        Command::Add(a) => add::run(&ctx, &ui, a),
        Command::Commit(a) => commit::run(&ctx, &ui, a),
        Command::Status(a) => status::run(&ctx, &ui, a),
        Command::Log(a) => log::run(&ctx, &ui, a),
        Command::Diff(a) => diff::run(&ctx, &ui, a),
        Command::Checkout(a) => checkout::run(&ctx, &ui, a),
        Command::Cat(a) => cat::run(&ctx, &ui, a),
        Command::Remote(a) => remote::run(&ctx, &ui, a),
        Command::Clone(a) => clone::run(&ctx, &ui, a),
        Command::Push(a) => push::run(&ctx, &ui, a),
        Command::Pull(a) => pull::run(&ctx, &ui, a),
    }
}
