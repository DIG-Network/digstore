# Design: World-Class digstore CLI UX

- **Date:** 2026-06-09
- **Status:** Approved (brainstorm) — pending spec review
- **Scope:** `crates/digstore-cli` only (plus its tests). No changes to the store
  format, compiler, host, or remote protocol.

## 1. Goal

Make the `digstore` CLI feel world-class: Git-familiar behavior, polished
output, helpful errors, and good discoverability — while staying correct,
scriptable (`--json`), and respectful of non-interactive use (pipes, CI,
`NO_COLOR`).

Trigger / poster-child gap: `digstore add -A` errors today (`add` takes a single
optional `[PATH]`). It must work like Git.

Output personality (chosen): **polished cargo/gh hybrid** — right-aligned colored
verbs, tasteful color, progress for long ops, next-step hints; plain and
uncolored when piped, `--json`, or `NO_COLOR`.

Working-directory model (chosen): **directory-aware (Git-like)** — digstore scans
the store directory (honoring `.digignore` and `.gitignore`); `add` stages files;
`status` reports staged / modified / untracked.

This is a **breaking change** to human-readable output and to `add` semantics.
The project is pre-1.0 (v0.1.0). `--json` output stays structured and stable for
scripts. Tests that assert on human output are updated.

## 2. Architecture

All presentation is centralized in a new `ui` module so no command writes raw
ANSI or makes its own TTY decisions.

```
crates/digstore-cli/src/
  ui/
    mod.rs        // `Ui` value + construction from flags/env
    theme.rs      // anstyle styles, symbols, verb formatting
    progress.rs   // indicatif wrappers (spinner / bar), no-op when quiet/json/non-tty
    render.rs     // status/log/diff/summary renderers (human + json)
  ...
```

### 2.1 The `Ui` value

```rust
pub struct Ui {
    color: bool,     // resolved: TTY && !NO_COLOR && --color != never, or --color always
    json: bool,
    quiet: bool,
    verbose: bool,
}
```

- Built once in `dispatch` from global flags + environment, then **passed by
  reference into every command** (replacing today's scattered `println!`).
- Color resolution precedence: `--color always|never` > `NO_COLOR` / `CLICOLOR=0`
  > `CLICOLOR_FORCE` > `stdout.is_terminal()`. `--json` ⇒ color off, progress off.
- Writes through `anstream::AutoStream` so styles are stripped automatically when
  the stream is not a terminal (defense in depth even if `color` is mis-resolved).

### 2.2 `Ui` API (illustrative)

```rust
impl Ui {
    fn verb(&self, label: &str, msg: impl Display);   // "   Staging  3 resources"
    fn success(&self, msg: impl Display);             // "✓ Committed  a1b2c3d"
    fn warn(&self, msg: impl Display);                // "! ..."
    fn hint(&self, next: impl Display);               // "  next: digstore push origin"
    fn item(&self, marker: Marker, text: impl Display); // "+ readme.md" etc.
    fn progress(&self, kind: ProgressKind, total: Option<u64>) -> Progress; // indicatif or no-op
    fn columns(&self, rows: &[[String; N]]);          // hand-aligned, no table dep
    fn emit_json<T: Serialize>(&self, value: &T);     // pretty JSON to stdout
}
```

`Marker` = `Staged(+)` (green) | `Modified(~)` (yellow) | `Untracked(?)` (dim) |
`Removed(-)` (red). Verbs are right-aligned to a fixed width (cargo style).

### 2.3 Dependencies (all mainstream, audited; pass `cargo deny`)

| Crate | Use |
|---|---|
| `anstream` + `anstyle` | TTY-aware colored output (already in clap's tree) |
| `indicatif` | progress bars / spinners |
| `clap_complete` | shell completions |
| `ignore` | directory walk honoring `.digignore` + `.gitignore` (ripgrep's crate) |
| `globset` | glob patterns for `add` |

No table crate (columns are hand-aligned) to keep the dependency surface small.
New deps are added to `Cargo.lock`; `deny.toml` already gates the supply chain.

### 2.4 Global flags (`cli.rs`)

Add `--color <auto|always|never>` (default `auto`) and `-q/--quiet`. Keep
`--json`, `-v/--verbose`, `--dig-dir`. All remain `global = true`.

## 3. Git-parity behavior

### 3.1 `add`

```
digstore add [PATHS]...  [-A|--all] [--dry-run] [--key <name>] [--discovery]
```

- `PATHS` is variadic: files, directories (recursively staged), or glob patterns
  (`globset`). `digstore add .` stages the current directory.
- `-A/--all` stages everything under the **store root** (the directory containing
  `.dig/`), regardless of CWD within it — like `git add -A`.
- `--dry-run` lists what *would* be staged and stages nothing.
- `--key <name>` is only valid with exactly one file path (rename that resource);
  with multiple/dir/glob it is an error with a helpful message.
- `--discovery` unchanged (stages the well-known discovery manifest).
- **Resource key** for each staged file = its path relative to the store root,
  normalized to forward slashes (so keys are portable and match `status`).
- Files whose content is byte-identical to what is already staged are reported as
  `unchanged` and not re-staged (idempotent).
- Output: one `+ <key> (<size>)` line per newly staged file, or a summary count
  when more than a threshold (e.g. 20); always a final summary + next-step hint.

### 3.2 Directory walking & ignore rules

A single helper builds an `ignore::WalkBuilder` rooted at the store root:

- Honors `.digignore` (custom ignore filename) **and** `.gitignore`
  (`git_ignore(true)`), plus global/parent gitignores.
- **Always** excludes the `.dig/` directory and the store's own artifacts.
- Symlinks are not followed by default; hidden files are included unless ignored
  (Git includes dotfiles in `add -A` except ignored ones — match that).

`.digignore` uses gitignore syntax and lives at the store root.

### 3.3 `status` (directory-aware)

```
digstore status [-s|--short]
```

Classifies every non-ignored file under the store root:

- **staged** — present in the staging area. Sub-state shown: new vs. changed-vs-
  committed.
- **modified** — present in the working dir, *not* staged, and its content differs
  from the current generation's committed content for that key.
- **untracked** — present in the working dir, not staged, and not a committed
  resource key.

Committed content for comparison is obtained through the existing local
serve+decrypt path (the same machinery `cat`/`checkout` use): for a candidate
key, derive the committed plaintext and compare SHA-256 against the working
file's SHA-256. (Phase-3 optimization may cache a per-resource plaintext digest
to avoid decrypting on every `status`; not required for correctness.)

Output: a header (`● generation N  <short root>` or `No commits yet`), then
grouped, counted sections with markers and a hint when there are untracked files.
`-s/--short` prints one line per change (`+`/`~`/`?` + key), Git-style.

### 3.4 Aliases & typo help

clap subcommand aliases: `st`→`status`, `ci`→`commit`, `co`→`checkout`. Enable
clap's suggestion feature so `digstore stauts` suggests `status`.

## 4. Output & theming

- **Verbs** (right-aligned, colored): `Staging`, `Chunking`, `Sealing`, `Committed`,
  `Cloning`, `Pulling`, `Pushing`, `Verifying`. Success line begins `✓` (green);
  failures `✗` (red); warnings `!` (yellow).
- **Markers:** `+` staged (green), `~` modified (yellow), `?` untracked (dim),
  `-` removed (red).
- **Progress** (`indicatif`): a spinner+bar during commit chunking (per-file) and
  during clone/pull/push byte transfer. Auto-disabled when `--json`, `--quiet`, or
  non-TTY.
- **Humanized values:** sizes as `KiB/MiB/GiB`; `log` timestamps as relative
  ("2 minutes ago") with absolute on `--verbose`.
- **`log`:** list of `generation N  <short root>  <relative time>  <size>`;
  `--oneline` for one-line entries; `--json` unchanged shape.
- **`diff`:** colored `+`/`-`/`~` per resource key.
- **Next-step hints** after `init` (→ `add`), `add` (→ `commit`), `commit`
  (→ `push`), `clone` (→ `cat`). Suppressed by `--quiet`/`--json`.

## 5. Errors, help & discoverability

- **Error rendering** (in `main.rs`, via `Ui`): `error: <message>` (red bold) on
  stderr, followed by `help: <hint>` (cyan) and optional `note:`. Each `CliError`
  variant maps to a concrete hint (e.g. `NoStore` → "run `digstore init` to create
  a store here"; `NonFastForward` → "run `digstore pull` first"). Exit codes are
  unchanged.
- **Per-command help:** every subcommand gets `long_about` + an `EXAMPLES` section
  (clap `after_help`) showing real invocations.
- **Completions:** `digstore completions <bash|zsh|fish|powershell|elvish>` prints
  a completion script via `clap_complete`.
- **`--version`:** shows crate version + git commit (from a build-time env, or
  "unknown") + module-format version.
- **`--json` everywhere:** audit every command; any lacking a JSON branch gets one
  with a documented shape.
- **Safety:** `--dry-run` on `add` (Phase 1) and `commit` (Phase 3). Any
  prompt-worthy destructive action gains `--yes` to bypass. (`clone` into a
  non-empty dir is already guarded.)

## 6. Phasing

One spec; implemented in slices, each ending green (`cargo fmt`, `clippy -D
warnings`, `cargo test --workspace`, and the CI `cargo deny` subset).

- **Phase 1 — core (the headline):** `ui` module + theme + color/TTY resolution;
  `add` with `[PATHS]...`/`-A`/`.`/globs/`--dry-run`/`.digignore`+`.gitignore`;
  directory-aware `status`; cargo-style errors + per-command help/examples. Fixes
  `add -A`.
- **Phase 2 — polish:** `indicatif` progress for commit/clone/pull/push; humanized
  `log`/`diff`; `completions`; `--color`/`--quiet`; aliases + typo suggestions;
  rich `--version`.
- **Phase 3 — extras:** `unstage` / `rm --cached`; `--dry-run` on `commit`;
  per-resource committed-digest cache for fast `status`; man pages.

## 7. Testing strategy

- **Unit:** `Ui` color resolution (flag/env/TTY matrix); verb/marker formatting
  (assert on uncolored output); ignore-walk filtering (`.dig/`, `.digignore`,
  `.gitignore`); glob expansion; relative-time and size humanizers.
- **Integration (`assert_cmd`):** `add -A`/`add .`/`add <glob>`/multiple paths;
  `--dry-run` stages nothing; `status` staged/modified/untracked classification on
  a scratch store; `completions <shell>` emits a script; error output contains the
  expected `help:` hint; `--json` output parses and is stable; `NO_COLOR`/piped
  output contains no ANSI.
- **Determinism:** color/progress never appear under `--json` or when not a TTY.

## 8. Out of scope

Store format, compiler, host runtime, and remote protocol are unchanged. No new
remote endpoints. The directory-aware model adds **no new persisted state** beyond
the existing staging area (Phase 3's digest cache, if built, is an optional local
cache).
