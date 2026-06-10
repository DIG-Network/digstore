//! Central CLI presentation. Build one `Ui` from flags + environment and pass it
//! into commands; never write raw ANSI or make TTY decisions elsewhere.

pub mod theme;

use std::io::{IsTerminal, Write};

use serde::Serialize;

use crate::ui::theme::{marker_line, verb_line, Marker};

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
    #[allow(dead_code)]
    verbose: bool,
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
        verbose: bool,
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
            verbose,
        }
    }

    /// Build from CLI flags, reading the real environment + TTY.
    pub fn from_flags(color: ColorChoice, json: bool, quiet: bool, verbose: bool) -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some()
            || std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false);
        let force = std::env::var_os("CLICOLOR_FORCE").is_some();
        Ui::resolve(
            color,
            json,
            quiet,
            verbose,
            std::io::stdout().is_terminal(),
            no_color,
            force,
        )
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
        let ui = Ui::resolve(ColorChoice::Always, true, false, false, true, false, false);
        assert!(!ui.color());
        assert!(ui.json());
    }

    #[test]
    fn never_disables_even_on_tty() {
        let ui = Ui::resolve(ColorChoice::Never, false, false, false, true, false, false);
        assert!(!ui.color());
    }

    #[test]
    fn auto_follows_tty() {
        assert!(Ui::resolve(ColorChoice::Auto, false, false, false, true, false, false).color());
        assert!(!Ui::resolve(ColorChoice::Auto, false, false, false, false, false, false).color());
    }

    #[test]
    fn no_color_env_wins_over_auto_tty() {
        let ui = Ui::resolve(ColorChoice::Auto, false, false, false, true, true, false);
        assert!(!ui.color());
    }

    #[test]
    fn clicolor_force_enables_without_tty() {
        let ui = Ui::resolve(ColorChoice::Auto, false, false, false, false, false, true);
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
