pub mod beacon;
pub mod branding;
pub mod cli;
pub mod commands;
pub mod config;
pub mod context;
mod continuation_guard;
pub mod dig_toml;
pub mod error;
pub mod ops;
pub mod output;
pub mod runtime;
pub mod templates;
pub mod ui;
pub mod workspace;

/// The file-stem of the binary as it was invoked (arg0), e.g. `digstore` or `digs`
/// (issue #434 — the `digs` alias). The extension (`.exe` on Windows) and any
/// directory prefix are stripped, so a `/usr/bin/digs` or `C:\...\digs.exe`
/// invocation both yield `"digs"`. This is what the CLI reports as its program name
/// in `--help`/`--version`/completions/`--help-json`, making the alias first-class
/// (each binary shows its own name rather than a hardcoded `"digstore"`). Falls back
/// to `"digstore"` when arg0 is somehow absent/empty.
pub fn invoked_bin_name() -> String {
    std::env::args_os()
        .next()
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::file_stem)
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "digstore".to_string())
}

/// The shared CLI entrypoint for BOTH the `digstore` and `digs` binaries (issue
/// #434). Kept here in the library — not duplicated in each `src/bin` shim — so the
/// two binaries are byte-for-byte the same command surface with ONE codepath.
///
/// Parses argv with the ACTUAL invoked binary name ([`invoked_bin_name`]) as both
/// the displayed program name and the usage `bin_name`, so `digs --help` shows
/// `digs` and `digstore --help` shows `digstore`. Never returns: it always exits the
/// process (mirroring a `fn main`).
pub fn run() -> ! {
    use clap::{CommandFactory, FromArgMatches};

    // `--help-json`: print the machine-readable CLI schema and exit, BEFORE clap
    // parses (so it works with no subcommand: `digstore --help-json`). Mirrors how
    // clap itself intercepts `--help`/`--version`. Agents/docs read the whole
    // command surface from this instead of scraping `--help`.
    if std::env::args().any(|a| a == "--help-json") {
        commands::completion::print_help_json();
        std::process::exit(0);
    }

    // Parse with the invoked binary's name as the program + bin name, so the alias
    // (`digs`) is first-class: its help/usage/version/errors all read `digs`, never a
    // hardcoded `digstore`, and never the raw arg0 (which may be an absolute path).
    // `get_matches()` still intercepts `--help`/`--version` and exits on a parse
    // error, using this name.
    //
    // `Command::name` requires `Into<Str>`, which this clap only satisfies for a
    // `&'static str`; the invoked name is computed at runtime, so we leak the tiny
    // stem to obtain a `'static` reference. This is a single, process-lifetime
    // allocation on the entrypoint of a short-lived CLI — never in a loop — so it is
    // not a meaningful leak. (`bin_name` takes `Into<String>`, so it takes the owned
    // value directly.)
    let bin = invoked_bin_name();
    let bin_static: &'static str = Box::leak(bin.clone().into_boxed_str());
    let matches = cli::Cli::command()
        .name(bin_static)
        .bin_name(bin)
        .get_matches();
    let cli = match cli::Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };

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
    let is_update = matches!(cli.command, cli::Command::Update(_));

    let ui = ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.non_interactive, cli.yes);
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
            // Honor --json: emit a structured {ok:false,error:{code,exit_code,
            // message,hint}} object to stdout for agents; human lines otherwise.
            ui.emit_error(&e);
            std::process::exit(e.exit_code());
        }
    }
}

/// Test-only helpers shared across `src/**` unit-test modules.
#[cfg(test)]
pub(crate) mod testutil {
    use std::sync::Mutex;

    /// ONE process-wide lock for every test that mutates the process-global
    /// `DIG_IDENTITY_DIR` env var (and its cousins read from the same dir,
    /// `DIG_NODE_URL`/`DIG_NODE_PORT`). This var is read by `ops::identity`,
    /// `ops::dighub`, and `ops::node` across SEPARATE test modules that all
    /// run in the SAME process (lib unit tests share one binary and run in
    /// parallel by default) — a per-module `Mutex` does NOT serialize across
    /// modules, since each one only guards itself against ITS OWN other
    /// tests. Every test that sets/removes `DIG_IDENTITY_DIR` (or a var read
    /// alongside it in the same test) MUST hold this lock for its entire body
    /// (set -> assert -> restore), otherwise a concurrent test in another
    /// module can observe a mid-mutation value and fail non-deterministically
    /// (this is exactly how `ops::identity`'s `identity_is_created_then_stable`
    /// was observed to flake against `ops::node`'s tests before this lock was
    /// unified).
    pub(crate) static DIG_IDENTITY_DIR_ENV_LOCK: Mutex<()> = Mutex::new(());
}
