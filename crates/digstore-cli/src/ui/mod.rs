//! Central CLI presentation. Build one `Ui` from flags + environment and pass it
//! into commands; never write raw ANSI or make TTY decisions elsewhere.

pub mod theme;

use std::io::{IsTerminal, Write};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

use crate::ui::theme::{marker_line, verb_line, Marker};

// ---------------------------------------------------------------------------
// Spinner — RAII handle wrapping indicatif's ProgressBar.
// ---------------------------------------------------------------------------

/// An animated spinner returned by [`Ui::spinner`].
///
/// Drop to clear the spinner line silently, or call [`Spinner::finish`] /
/// [`Spinner::finish_clear`] explicitly for controlled teardown.
pub struct Spinner {
    pb: ProgressBar,
}

impl Spinner {
    /// Stop the spinner and clear its line (no message printed).
    pub fn finish_clear(self) {
        self.pb.finish_and_clear();
    }

    /// Stop the spinner, clear its line, then let the caller print a result via
    /// the [`Ui`]. Pass the `Ui` and call `ui.success(msg)` / `ui.line(msg)` after
    /// if you want to print something — this just clears the spinner line.
    pub fn finish(self) {
        self.pb.finish_and_clear();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
    }
}

/// `--color` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct Ui {
    color: bool,
    json: bool,
    quiet: bool,
    /// Forced non-interactive (the `--non-interactive` flag): never prompt, fail fast.
    non_interactive: bool,
    /// Auto-approve confirmations (the `--yes`/`-y` flag).
    assume_yes: bool,
}

/// Plain capacity string: "47.2 MB staged · 52.8 MB free of 100.0 MB".
pub fn human_capacity(staged: u64, limit: u64) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    let free = limit.saturating_sub(staged);
    format!("{} staged · {} free of {}", mb(staged), mb(free), mb(limit))
}

impl Ui {
    /// Resolve color from the explicit choice, environment, json mode, and whether
    /// stdout is a terminal. `env_no_color`/`env_clicolor_force` are passed in so
    /// the logic is unit-testable without touching the real environment.
    pub fn resolve(
        color: ColorChoice,
        json: bool,
        quiet: bool,
        stdout_is_tty: bool,
        env_no_color: bool,
        env_clicolor_force: bool,
    ) -> Self {
        let color = if json {
            false
        } else {
            match color {
                ColorChoice::Always => true,
                ColorChoice::Never => false,
                ColorChoice::Auto => {
                    if env_no_color {
                        false
                    } else if env_clicolor_force {
                        true
                    } else {
                        stdout_is_tty
                    }
                }
            }
        };
        Ui {
            color,
            json,
            quiet,
            non_interactive: false,
            assume_yes: false,
        }
    }

    /// Build from CLI flags, reading the real environment + TTY.
    pub fn from_flags(
        color: ColorChoice,
        json: bool,
        quiet: bool,
        non_interactive: bool,
        assume_yes: bool,
    ) -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some()
            || std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false);
        let force = std::env::var_os("CLICOLOR_FORCE").is_some();
        let mut ui = Ui::resolve(
            color,
            json,
            quiet,
            std::io::stdout().is_terminal(),
            no_color,
            force,
        );
        ui.non_interactive = non_interactive;
        ui.assume_yes = assume_yes;
        ui
    }

    pub fn color(&self) -> bool {
        self.color
    }
    pub fn json(&self) -> bool {
        self.json
    }

    fn out(&self) -> anstream::AutoStream<std::io::Stdout> {
        anstream::AutoStream::auto(std::io::stdout())
    }

    /// Right-aligned colored verb line (cargo style).
    pub fn verb(&self, verb: &str, msg: impl std::fmt::Display) {
        if self.quiet || self.json {
            return;
        }
        let mut o = self.out();
        let _ = writeln!(o, "{}", verb_line(self.color, verb, &msg.to_string()));
    }

    /// Success line (`✓ ...`, green).
    pub fn success(&self, msg: impl std::fmt::Display) {
        if self.json {
            return;
        }
        let tick = theme::paint(
            self.color,
            anstyle::Style::new()
                .fg_color(Some(anstyle::AnsiColor::Green.into()))
                .bold(),
            "✓",
        );
        let mut o = self.out();
        let _ = writeln!(o, "{} {}", tick, msg);
    }

    /// A `+/~/?` item line.
    pub fn item(&self, marker: Marker, text: impl std::fmt::Display) {
        if self.quiet || self.json {
            return;
        }
        let mut o = self.out();
        let _ = writeln!(
            o,
            "  {}",
            marker_line(self.color, marker, &text.to_string())
        );
    }

    /// A next-step hint (suppressed when quiet/json).
    pub fn hint(&self, next: impl std::fmt::Display) {
        if self.quiet || self.json {
            return;
        }
        let dim = theme::paint(
            self.color,
            anstyle::Style::new().dimmed(),
            &format!("next: {next}"),
        );
        let mut o = self.out();
        let _ = writeln!(o, "  {}", dim);
    }

    /// Print a plain line (human mode only).
    pub fn line(&self, text: impl std::fmt::Display) {
        if self.json {
            return;
        }
        let mut o = self.out();
        let _ = writeln!(o, "{}", text);
    }

    /// Write a cargo-style `error:` + optional `help:` line to stderr.
    pub fn error(&self, e: &crate::error::CliError) {
        let mut err = anstream::AutoStream::auto(std::io::stderr());
        let label = theme::paint(
            self.color,
            anstyle::Style::new()
                .fg_color(Some(anstyle::AnsiColor::Red.into()))
                .bold(),
            "error:",
        );
        let _ = writeln!(err, "{} {}", label, e);
        if let Some(h) = e.hint() {
            let help = theme::paint(
                self.color,
                anstyle::Style::new().fg_color(Some(anstyle::AnsiColor::Cyan.into())),
                "help:",
            );
            let _ = writeln!(err, "{} {}", help, h);
        }
    }

    /// Emit pretty JSON to stdout (json mode).
    pub fn emit_json<T: Serialize>(&self, value: &T) {
        let mut o = self.out();
        let _ = writeln!(
            o,
            "{}",
            serde_json::to_string_pretty(value).expect("serialize")
        );
    }

    /// True only when we can safely prompt: not forced non-interactive (`--non-interactive`),
    /// both stdio ends are a terminal, and we are neither in `--quiet` nor `--json` mode.
    fn interactive(&self) -> bool {
        !self.non_interactive
            && !self.quiet
            && !self.json
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
    }

    /// Whether `--yes` was given (auto-approve confirmations).
    pub fn assume_yes(&self) -> bool {
        self.assume_yes
    }

    /// Public view of [`Ui::interactive`]: true only when we can safely prompt the
    /// user (interactive TTY, not `--non-interactive`/`--quiet`/`--json`).
    pub fn can_prompt(&self) -> bool {
        self.interactive()
    }

    /// A confirmation that MUST be satisfied to proceed (destructive/costly actions). `--yes`
    /// auto-approves. Interactive: a y/N prompt defaulting to No. Non-interactive without `--yes`:
    /// a hard error — so automation can never silently proceed past a dangerous action.
    pub fn confirm_or_fail(&self, prompt: &str) -> Result<(), crate::error::CliError> {
        if self.assume_yes {
            return Ok(());
        }
        if self.interactive() {
            if self.confirm(prompt, false) {
                return Ok(());
            }
            return Err(crate::error::CliError::InvalidArgument("aborted".into()));
        }
        Err(crate::error::CliError::InvalidArgument(format!(
            "{prompt} — pass --yes to proceed in non-interactive mode"
        )))
    }

    /// Require a value: interactive prompts for it; non-interactive (or an empty answer) errors,
    /// telling the caller which flag/argument to supply. `flag_hint` names that input
    /// (e.g. `"<name>"`, `"-m <message>"`).
    pub fn require_input(
        &self,
        prompt: &str,
        flag_hint: &str,
    ) -> Result<String, crate::error::CliError> {
        if let Some(v) = self.prompt_line(prompt, "") {
            return Ok(v);
        }
        Err(crate::error::CliError::InvalidArgument(format!(
            "{prompt} is required; pass {flag_hint} or run interactively"
        )))
    }

    /// Prompt for a single line of input. Returns the trimmed answer, or `None`
    /// when non-interactive (quiet/json/not-a-TTY) or the user accepts the
    /// default with an empty line — callers then fall back to their default.
    pub fn prompt_line(&self, prompt: &str, default: &str) -> Option<String> {
        if !self.interactive() {
            return None;
        }
        let mut o = self.out();
        if default.is_empty() {
            let _ = write!(o, "{prompt}: ");
        } else {
            let _ = write!(o, "{prompt} [{default}]: ");
        }
        let _ = o.flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return None;
        }
        let t = line.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }

    /// Prompts for a passphrase with hidden (non-echoed) input.
    /// Returns `None` when not attached to an interactive terminal.
    pub fn prompt_password(&self, prompt: &str) -> Option<String> {
        if !self.interactive() {
            return None;
        }
        rpassword::prompt_password(format!("{prompt}: ")).ok()
    }

    /// Yes/no prompt. Returns `default` when non-interactive or on empty input.
    pub fn confirm(&self, prompt: &str, default: bool) -> bool {
        if !self.interactive() {
            return default;
        }
        let hint = if default { "[Y/n]" } else { "[y/N]" };
        let mut o = self.out();
        let _ = write!(o, "{prompt} {hint} ");
        let _ = o.flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return default;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => true,
            "n" | "no" => false,
            _ => default,
        }
    }

    /// Return a bytes-progress bar for a transfer of `total_bytes`.
    ///
    /// Template: `msg [####----] 42.1/135.0 MB  1.2 MB/s  eta 1m23s`
    ///
    /// **Gating**: identical to [`Ui::spinner`] — hidden when `--json` is set,
    /// stdout is not a TTY, or color is disabled. The returned [`ProgressBar`]
    /// is always safe to call `set_position`/`finish_and_clear` on regardless.
    pub fn progress_bar(&self, total_bytes: u64, msg: &str) -> ProgressBar {
        let tty = std::io::stdout().is_terminal();
        if !tty || self.json || !self.color {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new(total_bytes);
        pb.set_style(
            ProgressStyle::with_template(
                "{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}  {bytes_per_sec}  eta {eta}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        pb.set_message(msg.to_owned());
        pb
    }

    /// Return an animated spinner that runs until the returned [`Spinner`] is
    /// dropped or explicitly finished.
    ///
    /// **Gating**: returns a hidden (no-op) spinner when `--json` is set, when
    /// stdout is not a TTY, or when color is disabled — so JSON/non-interactive
    /// output is never polluted.
    pub fn spinner(&self, msg: &str) -> Spinner {
        let tty = std::io::stdout().is_terminal();
        let pb = if !tty || self.json || !self.color {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(msg.to_owned());
            pb.enable_steady_tick(Duration::from_millis(90));
            pb
        };
        Spinner { pb }
    }

    /// Render staged/free/limit capacity: numbers always (unless `--json`), with a
    /// `[####····]` bar only when color is on and not quiet.
    pub fn capacity(&self, staged: u64, limit: u64) {
        if self.json {
            return;
        }
        let nums = human_capacity(staged, limit);
        if self.color && !self.quiet {
            let width = 18usize;
            let filled = if limit == 0 {
                0
            } else {
                ((staged as u128 * width as u128) / limit as u128) as usize
            };
            let filled = filled.min(width);
            let bar: String = core::iter::repeat_n('#', filled)
                .chain(core::iter::repeat_n('·', width - filled))
                .collect();
            let mut o = self.out();
            let _ = writeln!(o, "  {nums}  [{bar}]");
        } else {
            let mut o = self.out();
            let _ = writeln!(o, "  {nums}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_forces_color_off() {
        let ui = Ui::resolve(ColorChoice::Always, true, false, true, false, false);
        assert!(!ui.color());
        assert!(ui.json());
    }

    #[test]
    fn never_disables_even_on_tty() {
        let ui = Ui::resolve(ColorChoice::Never, false, false, true, false, false);
        assert!(!ui.color());
    }

    #[test]
    fn auto_follows_tty() {
        assert!(Ui::resolve(ColorChoice::Auto, false, false, true, false, false).color());
        assert!(!Ui::resolve(ColorChoice::Auto, false, false, false, false, false).color());
    }

    #[test]
    fn no_color_env_wins_over_auto_tty() {
        let ui = Ui::resolve(ColorChoice::Auto, false, false, true, true, false);
        assert!(!ui.color());
    }

    #[test]
    fn clicolor_force_enables_without_tty() {
        let ui = Ui::resolve(ColorChoice::Auto, false, false, false, false, true);
        assert!(ui.color());
    }

    #[test]
    fn human_capacity_is_plain_when_no_color() {
        let s = human_capacity(47_200_000, 100_000_000);
        assert!(s.contains("47.2 MB"));
        assert!(s.contains("52.8 MB free"));
        assert!(s.contains("100.0 MB"));
    }
}
