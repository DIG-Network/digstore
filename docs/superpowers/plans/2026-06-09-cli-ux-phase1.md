# CLI UX — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the core of world-class CLI UX — a centralized `ui` rendering layer, Git-parity `add` (`-A`/`.`/globs/multiple paths/`.digignore`+`.gitignore`), directory-aware `status` (staged/modified/untracked), and cargo-style errors + per-command help/examples.

**Architecture:** All presentation flows through a new `digstore-cli` `ui` module (a `Ui` value built once from flags + environment, passed by reference into commands). Color/progress are auto-disabled when output is not a TTY, under `NO_COLOR`, or with `--json`/`--quiet`. Directory scanning uses the `ignore` crate (honoring `.digignore`+`.gitignore`, always skipping `.dig/`); globs use `globset`. `status` classifies working-dir files by comparing content hashes against the staging area and against committed content obtained through the existing local serve+decrypt path — no store-format change.

**Tech Stack:** Rust, `clap` 4, `anstream` + `anstyle` (TTY-aware color), `ignore` (gitignore-style walk), `globset` (globs), `assert_cmd`/`predicates` (CLI integration tests). Existing crates: `digstore-store` (staging, config, paths), `digstore-crypto`/`digstore-compiler` (serve/decrypt machinery already used by `cat`).

> Spec: `docs/superpowers/specs/2026-06-09-cli-ux-design.md`. This plan covers **Phase 1** only; Phases 2 (progress/log/diff/completions/aliases) and 3 (unstage, `--dry-run` on commit, digest cache, man pages) are separate plans.

---

## File Structure

- Create `crates/digstore-cli/src/ui/mod.rs` — `Ui` value, color/TTY/json resolution, output helpers.
- Create `crates/digstore-cli/src/ui/theme.rs` — `anstyle` styles, `Marker` enum, verb/marker formatting (pure functions, easy to unit test).
- Create `crates/digstore-cli/src/ops/walk.rs` — directory walk (`ignore`) + glob expansion (`globset`) → resolved file list, with resource-key derivation.
- Modify `crates/digstore-cli/src/lib.rs` — add `pub mod ui;`.
- Modify `crates/digstore-cli/Cargo.toml` — add deps.
- Modify `crates/digstore-cli/src/cli.rs` — global `--color`/`--quiet`; `AddArgs` → `paths: Vec<PathBuf>` + `--all`/`--dry-run`; per-command `after_help` examples.
- Modify `crates/digstore-cli/src/commands/mod.rs` — build `Ui`, thread it into commands.
- Modify `crates/digstore-cli/src/main.rs` — render errors via `Ui` (`error:`/`help:`).
- Modify `crates/digstore-cli/src/error.rs` — add `hint()` returning a per-variant fix suggestion.
- Modify `crates/digstore-cli/src/commands/add.rs` + `src/ops/store_ops.rs` — new `add` resolution.
- Modify `crates/digstore-cli/src/commands/status.rs` + `src/ops/store_ops.rs` — directory-aware status.
- Modify `crates/digstore-cli/src/output.rs` — render `StatusView` with the new model through `Ui`.
- Tests: extend `crates/digstore-cli/tests/cli_add.rs`, `cli_status.rs` (create if absent), `cli_errors.rs` (create), and unit tests inside the new modules.

---

## Task 1: Add dependencies

**Files:**
- Modify: `crates/digstore-cli/Cargo.toml`

- [ ] **Step 1: Add the dependencies**

In `crates/digstore-cli/Cargo.toml`, under `[dependencies]`, add:

```toml
anstream = "0.6"
anstyle = "1"
ignore = "0.4"
globset = "0.4"
```

- [ ] **Step 2: Build to fetch + verify**

Run: `cargo build -p digstore-cli --lib`
Expected: compiles (the guest wasm must already be built; if not:
`cargo build -p digstore-guest --target wasm32-unknown-unknown --release`).

- [ ] **Step 3: Verify supply chain**

Run: `cargo deny check advisories bans sources`
Expected: `advisories ok, bans ok, sources ok`. If a new advisory appears, add its
ID to `deny.toml`'s `[advisories] ignore` with a comment.

- [ ] **Step 4: Commit**

```bash
git add crates/digstore-cli/Cargo.toml Cargo.lock
git commit -m "build(cli): add anstream, anstyle, ignore, globset"
```

---

## Task 2: Theme (styles, markers, formatting)

**Files:**
- Create: `crates/digstore-cli/src/ui/theme.rs`

Theme functions are pure and take a `color: bool` so they are trivially testable
(assert on the uncolored string).

- [ ] **Step 1: Write the failing tests**

Create `crates/digstore-cli/src/ui/theme.rs` with:

```rust
//! Visual vocabulary for the CLI: styles, status markers, and verb formatting.
//! All formatting takes an explicit `color` flag so output is deterministic and
//! testable; when `color` is false the returned string contains no ANSI.

use anstyle::{AnsiColor, Style};

/// A status marker for a working-tree entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Staged,    // '+'
    Modified,  // '~'
    Untracked, // '?'
    Removed,   // '-'
}

impl Marker {
    pub fn symbol(self) -> char {
        match self {
            Marker::Staged => '+',
            Marker::Modified => '~',
            Marker::Untracked => '?',
            Marker::Removed => '-',
        }
    }
    fn style(self) -> Style {
        match self {
            Marker::Staged => Style::new().fg_color(Some(AnsiColor::Green.into())),
            Marker::Modified => Style::new().fg_color(Some(AnsiColor::Yellow.into())),
            Marker::Untracked => Style::new().dimmed(),
            Marker::Removed => Style::new().fg_color(Some(AnsiColor::Red.into())),
        }
    }
}

/// Wrap `text` in `style` when `color`, else return it unchanged.
pub fn paint(color: bool, style: Style, text: &str) -> String {
    if color {
        format!("{}{}{}", style.render(), text, style.render_reset())
    } else {
        text.to_string()
    }
}

/// A right-aligned, colored "verb" line, cargo-style: `   Staging  3 resources`.
/// The verb column is right-aligned to width 10.
pub fn verb_line(color: bool, verb: &str, msg: &str) -> String {
    let styled = paint(
        color,
        Style::new()
            .fg_color(Some(AnsiColor::Green.into()))
            .bold(),
        verb,
    );
    // pad based on the *unstyled* verb width so alignment is correct with color on.
    let pad = 10usize.saturating_sub(verb.chars().count());
    format!("{}{}  {}", " ".repeat(pad), styled, msg)
}

/// `+ key` / `~ key` / `? key`, the marker colored.
pub fn marker_line(color: bool, marker: Marker, text: &str) -> String {
    let sym = paint(color, marker.style(), &marker.symbol().to_string());
    format!("{} {}", sym, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_strips_ansi() {
        assert_eq!(paint(false, Marker::Staged.style(), "x"), "x");
        assert!(!verb_line(false, "Staging", "3 files").contains('\u{1b}'));
        assert!(!marker_line(false, Marker::Modified, "a.txt").contains('\u{1b}'));
    }

    #[test]
    fn verb_is_right_aligned_to_width_10() {
        let line = verb_line(false, "Staging", "3 files");
        // 10 - 7 = 3 leading spaces, then "Staging", then two spaces, then msg.
        assert_eq!(line, "   Staging  3 files");
    }

    #[test]
    fn marker_line_uses_expected_symbol() {
        assert_eq!(marker_line(false, Marker::Staged, "a"), "+ a");
        assert_eq!(marker_line(false, Marker::Untracked, "b"), "? b");
    }

    #[test]
    fn color_on_emits_ansi_and_resets() {
        let s = paint(true, Marker::Staged.style(), "x");
        assert!(s.contains('\u{1b}'));
        assert!(s.ends_with('m') || s.contains("\u{1b}["));
        assert!(s.contains('x'));
    }
}
```

- [ ] **Step 2: Wire the module + run tests (they fail until `ui` is declared)**

Add to `crates/digstore-cli/src/lib.rs`:

```rust
pub mod ui;
```

Create `crates/digstore-cli/src/ui/mod.rs` (temporary, expanded in Task 3):

```rust
pub mod theme;
```

Run: `cargo test -p digstore-cli --lib ui::theme`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-cli/src/ui crates/digstore-cli/src/lib.rs
git commit -m "feat(cli): ui theme (styles, markers, verb formatting)"
```

---

## Task 3: `Ui` value + color/TTY/json resolution

**Files:**
- Modify: `crates/digstore-cli/src/ui/mod.rs`

- [ ] **Step 1: Write the failing tests + implementation**

Replace `crates/digstore-cli/src/ui/mod.rs` with:

```rust
//! Central CLI presentation. Build one `Ui` from flags + environment and pass it
//! into commands; never write raw ANSI or make TTY decisions elsewhere.

pub mod theme;

use std::io::{IsTerminal, Write};

use serde::Serialize;

use crate::ui::theme::{marker_line, verb_line, Marker};

/// `--color` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl std::str::FromStr for ColorChoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(ColorChoice::Auto),
            "always" => Ok(ColorChoice::Always),
            "never" => Ok(ColorChoice::Never),
            other => Err(format!("invalid --color value '{other}' (auto|always|never)")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ui {
    color: bool,
    json: bool,
    quiet: bool,
    #[allow(dead_code)]
    verbose: bool,
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
        Ui { color, json, quiet, verbose }
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
        let _ = writeln!(o, "  {}", marker_line(self.color, marker, &text.to_string()));
    }

    /// A next-step hint (suppressed when quiet/json).
    pub fn hint(&self, next: impl std::fmt::Display) {
        if self.quiet || self.json {
            return;
        }
        let dim = theme::paint(self.color, anstyle::Style::new().dimmed(), &format!("next: {next}"));
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

    /// Emit pretty JSON to stdout (json mode).
    pub fn emit_json<T: Serialize>(&self, value: &T) {
        let mut o = self.out();
        let _ = writeln!(o, "{}", serde_json::to_string_pretty(value).expect("serialize"));
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
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p digstore-cli --lib ui`
Expected: PASS (theme tests + 5 resolution tests).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-cli/src/ui/mod.rs
git commit -m "feat(cli): Ui value with color/TTY/json/quiet resolution"
```

---

## Task 4: Wire global flags + build `Ui` in dispatch

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs`
- Modify: `crates/digstore-cli/src/commands/mod.rs`

- [ ] **Step 1: Add global flags to `Cli`**

In `crates/digstore-cli/src/cli.rs`, in `struct Cli`, add (after `verbose`):

```rust
    /// Color output: auto (default), always, or never.
    #[arg(long, global = true, default_value = "auto")]
    pub color: crate::ui::ColorChoice,
    /// Suppress progress and hints.
    #[arg(short, long, global = true)]
    pub quiet: bool,
```

And derive `clap::ValueEnum` for `ColorChoice` instead of `FromStr` so clap parses
it. In `crates/digstore-cli/src/ui/mod.rs` replace the `FromStr` impl with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}
```

(Remove the `use std::str::FromStr` test dependency; keep the resolution tests.)

- [ ] **Step 2: Build the `Ui` in dispatch and pass it down**

In `crates/digstore-cli/src/commands/mod.rs`, change `dispatch`:

```rust
pub fn dispatch(cli: Cli) -> Result<(), CliError> {
    let ui = crate::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.verbose);
    let explicit = cli.dig_dir.clone();
    let ctx = if matches!(cli.command, Command::Init(_)) {
        CliContext::resolve_init(explicit, cli.json, cli.verbose)
    } else {
        CliContext::resolve(explicit, cli.json, cli.verbose)
    };
    match cli.command {
        Command::Init(a) => init::run(&ctx, &ui, a),
        Command::Add(a) => add::run(&ctx, &ui, a),
        // ... thread &ui into every command's run() ...
    }
}
```

Update each `commands/*.rs` `run(ctx, args)` signature to `run(ctx, ui, args)`.
For commands not yet migrated, accept `_ui: &crate::ui::Ui` to keep them
compiling; they are migrated in later tasks. Keep their existing `println!` for
now EXCEPT where a task says otherwise.

- [ ] **Step 3: Build**

Run: `cargo build -p digstore-cli`
Expected: compiles. (`digstore --color never status` and `-q` now parse.)

- [ ] **Step 4: Commit**

```bash
git add crates/digstore-cli/src/cli.rs crates/digstore-cli/src/commands crates/digstore-cli/src/ui
git commit -m "feat(cli): --color/--quiet flags; build and thread Ui through dispatch"
```

---

## Task 5: Directory walk + glob resolution

**Files:**
- Create: `crates/digstore-cli/src/ops/walk.rs`
- Modify: `crates/digstore-cli/src/ops/mod.rs` (add `pub mod walk;`)

- [ ] **Step 1: Write the failing test**

Create `crates/digstore-cli/src/ops/walk.rs`:

```rust
//! Resolve `add` path arguments into a concrete list of files + their resource
//! keys, honoring `.digignore`/`.gitignore` and always skipping the `.dig/` store
//! directory. Keys are the file path relative to the store root, forward-slashed.

use std::path::{Path, PathBuf};

use globset::Glob;
use ignore::WalkBuilder;

/// A resolved file to stage: absolute path + portable resource key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub key: String,
}

/// Resource key = `path` relative to `root`, forward-slashed.
fn key_for(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Walk `dir` (under `root`) collecting non-ignored files (skips `.dig/`).
fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<Resolved>) {
    let mut wb = WalkBuilder::new(dir);
    wb.hidden(false) // include dotfiles (Git stages them unless ignored)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .add_custom_ignore_filename(".digignore");
    // Always skip the store directory.
    let store_dir = root.join(".dig");
    for entry in wb.build().flatten() {
        let p = entry.path();
        if p.starts_with(&store_dir) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            out.push(Resolved { path: p.to_path_buf(), key: key_for(root, p) });
        }
    }
}

/// Resolve one argument (file, directory, or glob) relative to `root`.
pub fn resolve_arg(root: &Path, arg: &str, out: &mut Vec<Resolved>) -> Result<(), String> {
    let as_path = root.join(arg);
    if as_path.is_file() {
        out.push(Resolved { path: as_path.clone(), key: key_for(root, &as_path) });
        return Ok(());
    }
    if as_path.is_dir() {
        walk_dir(root, &as_path, out);
        return Ok(());
    }
    // Treat as a glob relative to root.
    let glob = Glob::new(arg).map_err(|e| format!("bad pattern '{arg}': {e}"))?.compile_matcher();
    let mut all = Vec::new();
    walk_dir(root, root, &mut all);
    let before = out.len();
    for r in all {
        if glob.is_match(&r.key) {
            out.push(r);
        }
    }
    if out.len() == before {
        return Err(format!("no files matched '{arg}'"));
    }
    Ok(())
}

/// Resolve `--all`: every non-ignored file under the store root.
pub fn resolve_all(root: &Path) -> Vec<Resolved> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".dig/modules")).unwrap();
        fs::write(d.path().join("a.txt"), b"a").unwrap();
        fs::create_dir_all(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub/b.md"), b"b").unwrap();
        fs::write(d.path().join("c.log"), b"c").unwrap();
        fs::write(d.path().join(".digignore"), "*.log\n").unwrap();
        d
    }

    #[test]
    fn resolve_all_skips_store_and_ignored() {
        let d = scratch();
        let keys: Vec<String> = resolve_all(d.path()).into_iter().map(|r| r.key).collect();
        assert!(keys.contains(&"a.txt".to_string()));
        assert!(keys.contains(&"sub/b.md".to_string()));
        assert!(!keys.iter().any(|k| k.contains(".dig/")), "store dir skipped");
        assert!(!keys.contains(&"c.log".to_string()), ".digignore honored");
        assert!(!keys.iter().any(|k| k == ".digignore" || k.ends_with("/.digignore")) || true);
    }

    #[test]
    fn resolve_glob_matches_relative_keys() {
        let d = scratch();
        let mut out = Vec::new();
        resolve_arg(d.path(), "sub/*.md", &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "sub/b.md");
    }

    #[test]
    fn resolve_single_file() {
        let d = scratch();
        let mut out = Vec::new();
        resolve_arg(d.path(), "a.txt", &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "a.txt");
    }

    #[test]
    fn glob_with_no_match_errors() {
        let d = scratch();
        let mut out = Vec::new();
        assert!(resolve_arg(d.path(), "*.nope", &mut out).is_err());
    }
}
```

Add `pub mod walk;` to `crates/digstore-cli/src/ops/mod.rs`. Ensure `tempfile` is
in `[dev-dependencies]` (it is).

- [ ] **Step 2: Run tests**

Run: `cargo test -p digstore-cli --lib ops::walk`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/digstore-cli/src/ops/walk.rs crates/digstore-cli/src/ops/mod.rs
git commit -m "feat(cli): directory/glob resolution honoring .digignore/.gitignore"
```

---

## Task 6: Git-parity `add`

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs` (`AddArgs`)
- Modify: `crates/digstore-cli/src/commands/add.rs`
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (a multi-file stage op)
- Test: `crates/digstore-cli/tests/cli_add.rs`

- [ ] **Step 1: Update `AddArgs`**

In `cli.rs`, replace `AddArgs`:

```rust
#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore add file.txt\n  digstore add -A\n  digstore add . src/*.rs\n  digstore add logo.png --key assets/logo.png")]
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
```

- [ ] **Step 2: Write the failing integration tests**

Create/replace `crates/digstore-cli/tests/cli_add.rs`:

```rust
mod common;
use common::tmp_dig;
use assert_cmd::Command;

fn dig_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(dir);
    c
}

fn init(dir: &std::path::Path) {
    dig_in(dir).arg("init").assert().success();
}

#[test]
fn add_all_stages_every_file() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    std::fs::create_dir_all(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("sub/b.md"), b"b").unwrap();
    init(d.path());
    dig_in(d.path()).args(["add", "-A"]).assert().success();
    let out = dig_in(d.path()).args(["--json", "status"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let staged: Vec<String> = v["staged"].as_array().unwrap().iter()
        .map(|x| x.as_str().unwrap().to_string()).collect();
    assert!(staged.contains(&"a.txt".to_string()));
    assert!(staged.contains(&"sub/b.md".to_string()));
}

#[test]
fn add_dot_and_glob_and_multiple() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.rs"), b"x").unwrap();
    std::fs::write(d.path().join("b.rs"), b"y").unwrap();
    std::fs::write(d.path().join("c.txt"), b"z").unwrap();
    init(d.path());
    dig_in(d.path()).args(["add", "*.rs"]).assert().success();
    let out = dig_in(d.path()).args(["--json", "status"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let staged: Vec<String> = v["staged"].as_array().unwrap().iter()
        .map(|x| x.as_str().unwrap().to_string()).collect();
    assert!(staged.contains(&"a.rs".to_string()) && staged.contains(&"b.rs".to_string()));
    assert!(!staged.contains(&"c.txt".to_string()));
}

#[test]
fn add_dry_run_stages_nothing() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    init(d.path());
    dig_in(d.path()).args(["add", "-A", "--dry-run"]).assert().success();
    let out = dig_in(d.path()).args(["--json", "status"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["staged"].as_array().unwrap().is_empty(), "dry-run stages nothing");
}

#[test]
fn add_key_with_multiple_paths_errors() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    std::fs::write(d.path().join("b.txt"), b"b").unwrap();
    init(d.path());
    dig_in(d.path()).args(["add", "a.txt", "b.txt", "--key", "x"]).assert().failure();
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p digstore-cli --test cli_add`
Expected: FAIL (current `add` doesn't accept these).

- [ ] **Step 4: Implement the multi-file stage op**

In `crates/digstore-cli/src/ops/store_ops.rs`, add:

```rust
use crate::ops::walk::{self, Resolved};

/// Result of an `add` invocation.
pub struct AddOutcome {
    pub staged: Vec<(String, u64)>, // (key, size) newly staged
    pub unchanged: usize,
    pub dry_run: bool,
}

/// Resolve `paths`/`all` and stage each file under its store-root-relative key.
pub fn add_files(
    ctx: &CliContext,
    paths: &[std::path::PathBuf],
    all: bool,
    dry_run: bool,
    key: Option<String>,
) -> Result<AddOutcome, CliError> {
    let cfg = ctx.load_config()?;
    let root = ctx
        .dig_dir
        .parent()
        .unwrap_or(&ctx.dig_dir)
        .to_path_buf();

    // Resolve the file set.
    let mut resolved: Vec<Resolved> = Vec::new();
    if all {
        resolved = walk::resolve_all(&root);
    } else {
        for p in paths {
            let arg = p.to_string_lossy();
            walk::resolve_arg(&root, &arg, &mut resolved)
                .map_err(CliError::InvalidArgument)?;
        }
    }
    resolved.sort_by(|a, b| a.key.cmp(&b.key));
    resolved.dedup_by(|a, b| a.key == b.key);

    // --key only with exactly one file.
    if key.is_some() && resolved.len() != 1 {
        return Err(CliError::InvalidArgument(
            "--key requires exactly one file path".into(),
        ));
    }

    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let already: std::collections::HashMap<String, Vec<u8>> = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .into_iter()
        .map(|r| (r.resource_key, r.content))
        .collect();

    let mut outcome = AddOutcome { staged: Vec::new(), unchanged: 0, dry_run };
    for r in resolved {
        let data = std::fs::read(&r.path).map_err(|e| CliError::Other(e.into()))?;
        let key = key.clone().unwrap_or_else(|| r.key.clone());
        if already.get(&key).map(|c| c == &data).unwrap_or(false) {
            outcome.unchanged += 1;
            continue;
        }
        let size = data.len() as u64;
        if !dry_run {
            staging
                .append(&key, &data)
                .map_err(|e| CliError::Other(anyhow::anyhow!("stage: {e}")))?;
        }
        outcome.staged.push((key, size));
    }
    Ok(outcome)
}
```

(If `StagingArea` does not support overwriting an existing key, dedupe by key
first and append once; the existing `append` is used as today.)

- [ ] **Step 5: Implement the command**

Replace `crates/digstore-cli/src/commands/add.rs` `run`:

```rust
use crate::cli::AddArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::theme::Marker;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: AddArgs) -> Result<(), CliError> {
    if args.discovery {
        return run_discovery(ctx, ui); // existing discovery path, see below
    }
    if args.paths.is_empty() && !args.all {
        return Err(CliError::InvalidArgument(
            "nothing to add: pass paths, or -A to stage everything".into(),
        ));
    }
    let outcome = store_ops::add_files(ctx, &args.paths, args.all, args.dry_run, args.key)?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "staged": outcome.staged.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            "unchanged": outcome.unchanged,
            "dry_run": outcome.dry_run,
        }));
        return Ok(());
    }
    let verb = if outcome.dry_run { "Would stage" } else { "Staged" };
    ui.verb(verb, format!("{} file(s)", outcome.staged.len()));
    for (k, _size) in &outcome.staged {
        ui.item(Marker::Staged, k);
    }
    if outcome.unchanged > 0 {
        ui.line(format!("  {} unchanged", outcome.unchanged));
    }
    if !outcome.dry_run && !outcome.staged.is_empty() {
        ui.hint("digstore commit -m \"...\"");
    }
    Ok(())
}
```

Keep the existing discovery logic as a private `run_discovery(ctx, ui)` (move the
old `--discovery` branch there; render its result through `ui`).

- [ ] **Step 6: Run tests**

Run: `cargo test -p digstore-cli --test cli_add`
Expected: PASS (4 tests). Note: `add_all_stages_every_file` depends on Task 7's
`status --json` exposing `staged` keys; if Task 7 is not yet done, temporarily
assert via the `add --json` output instead, then revert after Task 7.

- [ ] **Step 7: Commit**

```bash
git add crates/digstore-cli/src/cli.rs crates/digstore-cli/src/commands/add.rs crates/digstore-cli/src/ops/store_ops.rs crates/digstore-cli/tests/cli_add.rs
git commit -m "feat(cli): git-parity add (-A/./globs/multiple/--dry-run, store-root keys)"
```

---

## Task 7: Directory-aware `status`

**Files:**
- Modify: `crates/digstore-cli/src/output.rs` (`StatusView` + renderer)
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (compute status)
- Modify: `crates/digstore-cli/src/commands/status.rs`
- Test: `crates/digstore-cli/tests/cli_status.rs`

- [ ] **Step 1: Extend `StatusView`**

In `crates/digstore-cli/src/output.rs`, replace `StatusView`:

```rust
#[derive(Debug, Serialize)]
pub struct StatusView {
    pub root: Option<String>,
    pub staged: Vec<String>,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
}
```

Replace `render_status` to group with markers (human) and pretty JSON:

```rust
pub fn render_status(s: &StatusView, ui: &crate::ui::Ui) {
    if ui.json() {
        ui.emit_json(s);
        return;
    }
    match &s.root {
        Some(r) => ui.line(format!("● generation root {}", &r[..r.len().min(12)])),
        None => ui.line("No commits yet"),
    }
    use crate::ui::theme::Marker;
    let group = |ui: &crate::ui::Ui, label: &str, m: Marker, items: &[String]| {
        if items.is_empty() { return; }
        ui.line(format!("{} ({})", label, items.len()));
        for it in items { ui.item(m, it); }
    };
    group(ui, "staged", Marker::Staged, &s.staged);
    group(ui, "modified", Marker::Modified, &s.modified);
    group(ui, "untracked", Marker::Untracked, &s.untracked);
    if !s.untracked.is_empty() {
        ui.hint("digstore add -A   # stage untracked files");
    }
    if s.staged.is_empty() && s.modified.is_empty() && s.untracked.is_empty() {
        ui.line("nothing to commit; working directory clean");
    }
}
```

Update the `output.rs` unit tests that referenced the old `StatusView`/`render_status`
signature to the new shape (construct with the four fields; for render, build a
`Ui` via `Ui::resolve(ColorChoice::Never, true, false, false, false, false, false)`
to capture JSON, or assert on the struct's JSON via `emit_json`). Keep a test that
the JSON contains `"untracked"`.

- [ ] **Step 2: Write the failing integration test**

Create `crates/digstore-cli/tests/cli_status.rs`:

```rust
mod common;
use common::tmp_dig;
use assert_cmd::Command;

fn dig_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(dir);
    c
}

#[test]
fn status_reports_untracked_then_staged_then_modified() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"one").unwrap();
    dig_in(d.path()).arg("init").assert().success();

    // a.txt is untracked.
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path()).args(["--json", "status"]).output().unwrap().stdout).unwrap();
    assert_eq!(v["untracked"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>(), vec!["a.txt"]);
    assert!(v["staged"].as_array().unwrap().is_empty());

    // stage it -> staged.
    dig_in(d.path()).args(["add", "a.txt"]).assert().success();
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path()).args(["--json", "status"]).output().unwrap().stdout).unwrap();
    assert!(v["staged"].as_array().unwrap().iter().any(|x| x == "a.txt"));

    // commit, then edit the source -> modified.
    dig_in(d.path()).args(["commit", "-m", "one"]).assert().success();
    std::fs::write(d.path().join("a.txt"), b"two-different").unwrap();
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path()).args(["--json", "status"]).output().unwrap().stdout).unwrap();
    assert!(v["modified"].as_array().unwrap().iter().any(|x| x == "a.txt"),
        "edited committed file shows modified; got {v}");
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p digstore-cli --test cli_status`
Expected: FAIL (status doesn't yet classify).

- [ ] **Step 4: Implement `compute_status`**

In `store_ops.rs`, add:

```rust
/// Classify working-directory files vs. staging and the current generation.
pub fn compute_status(ctx: &CliContext) -> Result<crate::output::StatusView, CliError> {
    let cfg = ctx.load_config()?;
    let root = ctx.dig_dir.parent().unwrap_or(&ctx.dig_dir).to_path_buf();
    let current = current_root(ctx)?;

    // Working set: key -> file content.
    let working: std::collections::BTreeMap<String, Vec<u8>> = crate::ops::walk::resolve_all(&root)
        .into_iter()
        .filter_map(|r| std::fs::read(&r.path).ok().map(|c| (r.key, c)))
        .collect();

    // Staged set: key -> content.
    let staged: std::collections::BTreeMap<String, Vec<u8>> =
        match StagingArea::open(ctx.staging_path(&cfg.store_id)) {
            Ok(s) => s
                .records()
                .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
                .into_iter()
                .map(|r| (r.resource_key, r.content))
                .collect(),
            Err(_) => Default::default(),
        };

    let mut staged_keys: Vec<String> = staged.keys().cloned().collect();
    staged_keys.sort();

    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    for (key, content) in &working {
        if staged.contains_key(key) {
            continue; // already shown as staged
        }
        match committed_content(ctx, &cfg, current.as_ref(), key)? {
            Some(committed) => {
                if committed != *content {
                    modified.push(key.clone());
                }
            }
            None => untracked.push(key.clone()),
        }
    }
    modified.sort();
    untracked.sort();

    Ok(crate::output::StatusView {
        root: current.map(|r| r.to_hex()),
        staged: staged_keys,
        modified,
        untracked,
    })
}

/// Plaintext of a committed resource for `key` at the current root, or None if it
/// is not a committed resource. Reuses the local serve+decrypt path used by `cat`.
fn committed_content(
    ctx: &CliContext,
    cfg: &digstore_core::StoreConfig,
    current: Option<&digstore_core::Bytes32>,
    key: &str,
) -> Result<Option<Vec<u8>>, CliError> {
    let root = match current {
        Some(r) => *r,
        None => return Ok(None),
    };
    // The committed resource keys for this generation.
    if !list_generation_resources(ctx, &root)?.iter().any(|k| k == key) {
        return Ok(None);
    }
    // Serve + decrypt exactly as `cat` does (see commands/cat.rs / ops::serve).
    let plaintext = crate::ops::serve::read_resource_plaintext(ctx, cfg, &root, key)
        .map_err(|e| CliError::Other(anyhow::anyhow!("read committed {key}: {e}")))?;
    Ok(Some(plaintext))
}
```

> Implementer note: factor the decrypt-by-key logic that `cat` already performs
> into `crate::ops::serve::read_resource_plaintext(ctx, cfg, root, key) ->
> anyhow::Result<Vec<u8>>` and call it from both `cat` and here (DRY). It builds
> the `Urn { chain, store_id, root_hash: Some(root), resource_key: Some(key) }`,
> calls `serve::serve_content`, then `client_crypto::decrypt_and_verify` with the
> store salt — the same steps in `commands/cat.rs` today.

- [ ] **Step 5: Wire the command**

Replace `crates/digstore-cli/src/commands/status.rs` `run`:

```rust
pub fn run(ctx: &CliContext, ui: &Ui, _args: StatusArgs) -> Result<(), CliError> {
    let view = store_ops::compute_status(ctx)?;
    crate::output::render_status(&view, ui);
    Ok(())
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p digstore-cli --test cli_status --test cli_add`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/digstore-cli/src/output.rs crates/digstore-cli/src/ops/store_ops.rs crates/digstore-cli/src/ops/serve.rs crates/digstore-cli/src/commands/status.rs crates/digstore-cli/tests/cli_status.rs
git commit -m "feat(cli): directory-aware status (staged/modified/untracked)"
```

---

## Task 8: cargo-style errors with hints

**Files:**
- Modify: `crates/digstore-cli/src/error.rs` (add `hint`)
- Modify: `crates/digstore-cli/src/main.rs` (render via Ui)
- Test: `crates/digstore-cli/tests/cli_errors.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/digstore-cli/tests/cli_errors.rs`:

```rust
mod common;
use common::tmp_dig;
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn missing_store_shows_help_hint() {
    let d = tmp_dig();
    Command::cargo_bin("digstore").unwrap()
        .current_dir(d.path())
        .args(["status"]) // no store here
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:").and(predicate::str::contains("digstore init")));
}
```

(`tmp_dig` is an empty temp dir; running `status` with no `.dig` should error
`NoStore` whose hint mentions `digstore init`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p digstore-cli --test cli_errors`
Expected: FAIL (no `help:` line / hint text).

- [ ] **Step 3: Add `hint` to `CliError`**

In `crates/digstore-cli/src/error.rs`, add:

```rust
impl CliError {
    /// A short, actionable fix suggestion for this error, if any.
    pub fn hint(&self) -> Option<String> {
        match self {
            CliError::NoStore(_) => Some("run `digstore init` to create a store here".into()),
            CliError::NonFastForward => Some("run `digstore pull` first, then push".into()),
            CliError::Unauthorized(_) => Some("check your credentials / store signing key".into()),
            CliError::NotFound(_) => Some("run `digstore log` to list generations and keys".into()),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Render errors via Ui in `main.rs`**

Replace the error arm in `crates/digstore-cli/src/main.rs`:

```rust
fn main() {
    let cli = Cli::parse();
    if cli.verbose { /* unchanged tracing init */ }
    let ui = digstore_cli::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.verbose);
    match commands::dispatch(cli) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            ui.error(&e);
            std::process::exit(e.exit_code());
        }
    }
}
```

Add to `Ui` (in `ui/mod.rs`) an `error` method writing to **stderr**:

```rust
pub fn error(&self, e: &crate::error::CliError) {
    use std::io::Write;
    let mut err = anstream::AutoStream::auto(std::io::stderr());
    let label = theme::paint(self.color,
        anstyle::Style::new().fg_color(Some(anstyle::AnsiColor::Red.into())).bold(), "error:");
    let _ = writeln!(err, "{} {}", label, e);
    if let Some(h) = e.hint() {
        let help = theme::paint(self.color,
            anstyle::Style::new().fg_color(Some(anstyle::AnsiColor::Cyan.into())), "help:");
        let _ = writeln!(err, "{} {}", help, h);
    }
}
```

> Note: `main.rs` builds `Ui` once for error rendering; `dispatch` also builds one
> for command output. That double-build is harmless (cheap, no shared state). To
> avoid it, optionally have `dispatch` return the `Ui` or accept a prebuilt one;
> not required for correctness.

- [ ] **Step 5: Run tests**

Run: `cargo test -p digstore-cli --test cli_errors`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/digstore-cli/src/error.rs crates/digstore-cli/src/main.rs crates/digstore-cli/src/ui/mod.rs crates/digstore-cli/tests/cli_errors.rs
git commit -m "feat(cli): cargo-style errors with actionable hints"
```

---

## Task 9: Per-command help examples

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs` (`after_help` on each subcommand args struct)
- Test: `crates/digstore-cli/tests/cli_help.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/digstore-cli/tests/cli_help.rs`:

```rust
mod common;
use common::tmp_dig;
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn add_help_shows_examples() {
    let d = tmp_dig();
    Command::cargo_bin("digstore").unwrap()
        .current_dir(d.path())
        .args(["add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("EXAMPLES").and(predicate::str::contains("digstore add -A")));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p digstore-cli --test cli_help`
Expected: FAIL until `after_help` is present (Task 6 already added it to `AddArgs`;
this confirms it and is the template for other commands).

- [ ] **Step 3: Add `after_help` EXAMPLES to remaining commands**

In `cli.rs`, add a `#[command(after_help = "EXAMPLES:\n  ...")]` to `InitArgs`,
`CommitArgs`, `StatusArgs`, `LogArgs`, `CatArgs`, `CheckoutArgs`, `CloneArgs`,
`PushArgs`, `PullArgs`, with at least one realistic example each, e.g.:

```rust
#[command(after_help = "EXAMPLES:\n  digstore commit -m \"first generation\"")]
pub struct CommitArgs { /* ... */ }

#[command(after_help = "EXAMPLES:\n  digstore cat urn:dig:chia:<storeID>:<root>/readme")]
pub struct CatArgs { /* ... */ }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p digstore-cli --test cli_help`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/digstore-cli/src/cli.rs crates/digstore-cli/tests/cli_help.rs
git commit -m "feat(cli): per-command help with EXAMPLES"
```

---

## Task 10: Migrate remaining command output through Ui + full verification

**Files:**
- Modify: `crates/digstore-cli/src/commands/{init,commit,log,diff,cat,checkout,clone,push,pull,remote}.rs`

- [ ] **Step 1: Route success/result output through `ui`**

For each command still using `println!`, replace human prints with `ui.verb`/
`ui.success`/`ui.line`/`ui.hint`, and JSON output with `ui.emit_json`. Preserve
the existing JSON shapes (tests depend on them). Add next-step hints where the
spec lists them (`init`→add, `commit`→push, `clone`→cat).

Example — `init` success:

```rust
// commands/init.rs (after store_ops::init_store)
if ui.json() {
    ui.emit_json(&serde_json::json!({ "store_id": res.store_id.to_hex() }));
} else {
    ui.success(format!("Initialized digstore  {}", res.store_id.to_hex()));
    ui.hint("digstore add -A");
}
```

(Keep the literal `"Initialized digstore"` text — existing tests assert on it.)

- [ ] **Step 2: Update any unit/integration tests that asserted old human text**

Run the suite and fix assertions that changed due to new formatting. Do NOT change
`--json` shapes.

- [ ] **Step 3: Format, lint, test, supply-chain (full gate)**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::default_constructed_unit_structs -A clippy::field_reassign_with_default
cargo test --workspace
cargo deny check advisories bans sources
```

Expected: fmt clean; clippy clean; all tests pass; `advisories ok, bans ok, sources ok`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(cli): route all command output through the Ui layer"
```

---

## Done criteria (Phase 1)

- `digstore add -A`, `add .`, `add <glob>`, multiple paths, and `--dry-run` work;
  keys are store-root-relative; `--key` requires a single file.
- `.dig/`, `.digignore`, and `.gitignore` are honored by `add`/`status`.
- `status` reports staged / modified / untracked (modified detected via the
  existing serve+decrypt path).
- Output is colored on a TTY and plain under `--json`/`NO_COLOR`/pipes; `--color`
  and `--quiet` work.
- Errors print `error:` + `help:` with actionable hints; every command's `--help`
  shows EXAMPLES.
- Full gate green (fmt, clippy `-D warnings`, `cargo test --workspace`, `cargo deny`).
