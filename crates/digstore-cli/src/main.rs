use clap::Parser;
use digstore_cli::beacon;
use digstore_cli::cli::{Cli, Command};
use digstore_cli::commands;

fn main() {
    let cli = Cli::parse();
    if cli.verbose {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "digstore=debug".into()),
            )
            .try_init();
    }
    // Capture the flags the post-command beacon needs before `cli` is consumed.
    // Skip the beacon for `update` itself (it already talks to GitHub).
    let (json, quiet) = (cli.json, cli.quiet);
    let is_update = matches!(cli.command, Command::Update(_));

    let ui = digstore_cli::ui::Ui::from_flags(
        cli.color,
        cli.json,
        cli.quiet,
        cli.non_interactive,
        cli.yes,
    );
    match commands::dispatch(cli) {
        Ok(()) => {
            // Best-effort, throttled, fail-safe update notice. Runs only after a
            // successful command and never affects this command's behavior.
            if !is_update {
                beacon::maybe_notify(json, quiet);
            }
            std::process::exit(0);
        }
        Err(e) => {
            ui.error(&e);
            std::process::exit(e.exit_code());
        }
    }
}
