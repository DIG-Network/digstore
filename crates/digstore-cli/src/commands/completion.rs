//! `digstore completion <shell>` — shell completion scripts, plus the
//! machine-readable CLI surface (`--help-json` schema + generated man pages).
//!
//! These are the "30-command CLI polish" surfaces from roadmap #27: tab
//! completion for daily use, and an agent-/docs-extractable description of the
//! whole command tree so the documentation never drifts from the binary.
//!
//! All three are derived from the SAME `clap::Command` (`Cli::command()`), so
//! they stay in lockstep with the actual flags automatically — there is no
//! second source of truth to keep updated.

use std::io;

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::error::CliError;
use crate::ui::Ui;

/// `digstore completion <shell>`: write the completion script to stdout.
pub fn run(_ui: &Ui, shell: Shell) -> Result<(), CliError> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut io::stdout());
    Ok(())
}

/// `digstore --help-json` (intercepted in `main`): print the WHOLE invocation
/// contract as JSON — every command, its aliases + one-line `about` + its args
/// (with value-enum `choices`, `default`, and `value_name`), the global flags
/// (documented once at the root), AND the differentiated exit-code table. Built
/// straight from the clap model + [`CliError::exit_code_table`] so it always
/// matches the binary's real surface. One introspection call yields everything an
/// agent needs to invoke the CLI and branch on its failures.
pub fn print_help_json() {
    let cmd = Cli::command();
    // The global flags (inherited by every subcommand) — documented once here so the
    // per-command `args` lists stay focused on command-specific flags.
    let globals: Vec<serde_json::Value> = cmd
        .get_arguments()
        .filter(|a| a.is_global_set())
        .map(arg_json)
        .collect();
    let exit_codes: Vec<serde_json::Value> = crate::error::CliError::exit_code_table()
        .iter()
        .map(|(code, exit, meaning)| {
            serde_json::json!({ "code": code, "exit_code": exit, "meaning": meaning })
        })
        .collect();
    let json = serde_json::json!({
        "name": cmd.get_name(),
        "version": env!("CARGO_PKG_VERSION"),
        "about": cmd.get_about().map(|s| s.to_string()),
        "globals": globals,
        "commands": cmd.get_subcommands().map(subcommand_json).collect::<Vec<_>>(),
        "exit_codes": exit_codes,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}

/// Describe one subcommand (name, aliases, about, and its non-global args).
fn subcommand_json(c: &clap::Command) -> serde_json::Value {
    let args: Vec<serde_json::Value> = c
        .get_arguments()
        // Skip the inherited globals (--json/--verbose/…); they are listed once at
        // the root under `globals` instead of cluttering every command entry.
        .filter(|a| !a.is_global_set())
        .map(arg_json)
        .collect();
    serde_json::json!({
        "name": c.get_name(),
        "aliases": c.get_visible_aliases().collect::<Vec<_>>(),
        "about": c.get_about().map(|s| s.to_string()),
        "args": args,
    })
}

/// Describe one argument completely: its long/short flags (or that it is
/// positional), whether it takes a value, its value-enum `choices`, its `default`,
/// its `value_name`, and its help text — so an agent learns the full per-arg
/// contract from `--help-json` alone.
fn arg_json(a: &clap::Arg) -> serde_json::Value {
    // Value-enum possible values (e.g. completion shells, --color auto/always/never).
    let choices: Vec<String> = a
        .get_possible_values()
        .iter()
        .map(|pv| pv.get_name().to_string())
        .collect();
    // Default value(s), if any (e.g. --wait-timeout 300, --color auto).
    let defaults: Vec<String> = a
        .get_default_values()
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    // The placeholder shown in help for the value (e.g. WRITER_SEED, DIG).
    let value_name = a
        .get_value_names()
        .and_then(|names| names.first().map(|s| s.to_string()));
    serde_json::json!({
        "id": a.get_id().as_str(),
        "long": a.get_long(),
        "short": a.get_short().map(|c| c.to_string()),
        "positional": a.is_positional(),
        "takes_value": a.get_num_args().map(|n| n.takes_values()).unwrap_or(false),
        "required": a.is_required_set(),
        "choices": choices,
        "default": defaults,
        "value_name": value_name,
        "help": a.get_help().map(|s| s.to_string()),
    })
}

/// Generate troff man pages for the root command + every subcommand into `out_dir`
/// (creating it). Returns the list of written file paths. Used by docs tooling
/// (roadmap #27/#57) so `docs.dig.net` can render the CLI reference from the
/// binary itself. Exposed as a library fn (no dedicated subcommand) so the docs
/// build can call it without shipping a user-facing verb.
pub fn generate_man_pages(out_dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, CliError> {
    std::fs::create_dir_all(out_dir).map_err(|e| CliError::Other(e.into()))?;
    let cmd = Cli::command();
    let mut written = Vec::new();

    // The root page.
    let root_path = out_dir.join("digstore.1");
    write_man(&cmd, &root_path)?;
    written.push(root_path);

    // One page per subcommand: `digstore-<sub>.1`. The page is built from the
    // subcommand's own clap model (so its flags/about are accurate); the
    // `digstore-<sub>` convention lives in the FILE name (renaming the in-memory
    // command is not portable across clap's `Str` conversions).
    for sub in cmd.get_subcommands() {
        let file = out_dir.join(format!("digstore-{}.1", sub.get_name()));
        write_man(sub, &file)?;
        written.push(file);
    }
    Ok(written)
}

fn write_man(cmd: &clap::Command, path: &std::path::Path) -> Result<(), CliError> {
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)
        .map_err(|e| CliError::Other(anyhow::anyhow!("render man page: {e}")))?;
    std::fs::write(path, buf).map_err(|e| CliError::Other(e.into()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_json_lists_every_command_and_flags() {
        // Build the schema the same way `print_help_json` does and assert it
        // covers the headline commands + a known flag, so docs/agents get the
        // full surface.
        let cmd = Cli::command();
        let names: Vec<String> = cmd
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        for expected in [
            "deploy",
            "new",
            "dev",
            "doctor",
            "setup",
            "link",
            "completion",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
        // `deploy` exposes `--if-changed` and `--dry-run` in the schema.
        let deploy = cmd
            .get_subcommands()
            .find(|c| c.get_name() == "deploy")
            .unwrap();
        let v = subcommand_json(deploy);
        let longs: Vec<String> = v["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["long"].as_str().map(|s| s.to_string()))
            .collect();
        assert!(longs.contains(&"if-changed".to_string()));
        assert!(longs.contains(&"dry-run".to_string()));
    }

    #[test]
    fn help_json_includes_globals_and_exit_codes() {
        // The full machine contract: globals (the inherited flags) + the exit-code
        // table are present and complete, so one --help-json call yields everything.
        let cmd = Cli::command();
        let globals: Vec<String> = cmd
            .get_arguments()
            .filter(|a| a.is_global_set())
            .filter_map(|a| a.get_long().map(|s| s.to_string()))
            .collect();
        // The headline globals are surfaced.
        for g in ["json", "verbose", "quiet", "color", "store"] {
            assert!(globals.contains(&g.to_string()), "missing global --{g}");
        }
        // The exit-code table mirrors error.rs and includes the success row + a
        // differentiated non-zero code.
        let table = crate::error::CliError::exit_code_table();
        assert!(table.iter().any(|(c, x, _)| *c == "OK" && *x == 0));
        assert!(table
            .iter()
            .any(|(c, x, _)| *c == "INSUFFICIENT_FUNDS" && *x == 12));
    }

    #[test]
    fn arg_json_exposes_choices_default_and_value_name() {
        let cmd = Cli::command();
        // --color is a value-enum global with a default of "auto".
        let color = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some("color"))
            .unwrap();
        let v = arg_json(color);
        let choices: Vec<String> = v["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        assert!(choices.contains(&"auto".to_string()));
        assert!(choices.contains(&"never".to_string()));
        let defaults: Vec<String> = v["default"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        assert!(defaults.contains(&"auto".to_string()), "color default=auto");

        // `commit --writer-key` carries a value_name (WRITER_SEED).
        let commit = cmd.find_subcommand("commit").unwrap();
        let wk = commit
            .get_arguments()
            .find(|a| a.get_long() == Some("writer-key"))
            .unwrap();
        assert_eq!(arg_json(wk)["value_name"].as_str(), Some("WRITER_SEED"));
    }

    #[test]
    fn setup_schema_has_auth_alias() {
        let cmd = Cli::command();
        let setup = cmd
            .get_subcommands()
            .find(|c| c.get_name() == "setup")
            .unwrap();
        let v = subcommand_json(setup);
        let aliases: Vec<String> = v["aliases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        assert!(aliases.contains(&"auth".to_string()));
    }

    #[test]
    fn generate_man_pages_writes_root_and_subcommands() {
        let td = tempfile::tempdir().unwrap();
        let written = generate_man_pages(td.path()).unwrap();
        // The root page + at least the headline subcommand pages exist.
        assert!(td.path().join("digstore.1").exists());
        assert!(td.path().join("digstore-deploy.1").exists());
        assert!(td.path().join("digstore-completion.1").exists());
        assert!(written.len() > 5, "one page per command");
    }
}
