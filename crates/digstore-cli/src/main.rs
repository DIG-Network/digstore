use clap::Parser;
use digstore_cli::cli::Cli;
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
    let ui = digstore_cli::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.verbose);
    match commands::dispatch(cli) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            ui.error(&e);
            std::process::exit(e.exit_code());
        }
    }
}
