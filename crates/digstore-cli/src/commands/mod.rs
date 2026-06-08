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
    let ctx = CliContext::resolve(cli.dig_dir.clone(), cli.json, cli.verbose);
    match cli.command {
        Command::Init(a) => init::run(&ctx, a),
        Command::Add(a) => add::run(&ctx, a),
        Command::Commit(a) => commit::run(&ctx, a),
        Command::Status(a) => status::run(&ctx, a),
        Command::Log(a) => log::run(&ctx, a),
        Command::Diff(a) => diff::run(&ctx, a),
        Command::Checkout(a) => checkout::run(&ctx, a),
        Command::Cat(a) => cat::run(&ctx, a),
        Command::Remote(a) => remote::run(&ctx, a),
        Command::Clone(a) => clone::run(&ctx, a),
        Command::Push(a) => push::run(&ctx, a),
        Command::Pull(a) => pull::run(&ctx, a),
    }
}
