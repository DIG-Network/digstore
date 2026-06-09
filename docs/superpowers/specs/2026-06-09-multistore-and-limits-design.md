# Design: Multi-Store Workspaces, Per-Store Size Cap, and Staging Management

- **Date:** 2026-06-09
- **Status:** Approved (brainstorm) — pending spec review
- **Builds on:** the shipped CLI UX Phase 1 (`2026-06-09-cli-ux-phase1.md`) and the
  `.dig` store-module extension. Reshapes the on-disk layout and makes every
  command store-aware.

## 1. Goals

1. **Multiple stores per project**, managed with clean, human-friendly UX
   (kubectl/git-context style): create named stores, list them, switch the active
   one, override per command.
2. **Per-store hard cap of 100 MB** (100,000,000 bytes), enforced at **stage**
   time — `add` refuses to stage beyond it with a clear error (no sharding).
3. **Stores up to 100 MB actually work** (commit + serve): raise the module
   memory ceiling from 16 MiB and make the guest heap sit **above** the
   (now-large) data section so serving never corrupts the embedded pool.
4. **Staging management:** `digstore unstage` (clear staged) and `digstore staged`
   (list staged).
5. **Configurable operating directory (content root).** Unlike Git, digstore
   captures **build output**, not repo files. The `.dig/` workspace may sit at the
   project root while the files a store captures live elsewhere (e.g. `dist/`).
   Each store has a per-store *content root*; `add`/`urn` operate on it and resource
   keys are relative to it. A one-shot `-C/--cwd` flag overrides it per command.

## 2. Multi-store workspace

### 2.1 Layout

`<project>/.dig/` becomes a **workspace** holding any number of named stores:

```text
<project>/.dig/
  workspace.toml               # active store + name -> store_id registry
  stores/
    <name>/                    # one full store per name
      config.toml              # StoreConfig (store_id, visibility, max_size=100_000_000, content_root)
      {store_id}.staging.bin
      roots.log
      generations/{roothash}/{manifest.json, chunks/...}
      modules/{store_id}-{root}.dig
      signing_key.bin          # 0600
      trusted_keys.json
      secret_salt.hex          # 0600, private stores only
```

`workspace.toml`:

```toml
active = "docs"

[stores]
docs = "ab12…64hex…"
site = "cd34…64hex…"
```

`.gitignore` continues to ignore `.dig/` (the init helper is unchanged in intent).

### 2.2 Store names

Human-friendly identifiers. Validation: non-empty; only `[A-Za-z0-9._-]`; not `.`
or `..`; no path separators. The 64-hex store id remains the cryptographic
identity (`SHA-256(pubkey)`, §20.1) and is what URNs use; the name is a local
alias recorded in `workspace.toml`.

### 2.3 Store selection (precedence)

For every command except `init`, the **target store** is resolved as:

1. `--store <name>` global flag, if given (error if the name is unknown).
2. else the workspace `active` store.
3. else, if exactly one store exists, that store (implicit).
4. else error: `no store selected: use --store <name>, set one with `digstore use <name>`, or create one with `digstore init <name>``.

`init` resolves the **workspace** (`.dig/` in the CWD, created if absent), not a
store.

### 2.4 New / changed commands

- `digstore init [name] [--dir <path>]` — create store `<name>` under
  `.dig/stores/<name>/` (default name `default` when omitted), register it in
  `workspace.toml`, and set it active if it is the first store. `--dir` records the
  store's **content root** (when omitted it is left unset → the operating directory
  defaults to the CWD per command, §2.8); commonly set to the build output dir,
  e.g. `digstore init site --dir dist`.
  Errors if `<name>` already exists. The `.gitignore` convenience (ignore `.dig/`)
  runs once at workspace creation.
- `digstore stores` — list stores: `*` active marker, name, short store id,
  content root (relative to project), current root (short) or `(empty)`, and
  used/limit. `--json` emits an array of `{name, store_id, active, content_root,
  current_root, generations, size_bytes, staged_bytes, limit_bytes}`.
- `digstore use <name>` — set `workspace.toml` `active = <name>` (error if
  unknown). Prints the new active store and its content root.
- `digstore dir [<path>]` — with no arg, print the selected store's content root;
  with a path, set it (persisted to that store's `config.toml`). Validates the
  path exists (a warning, not an error, if absent — build dirs may not exist yet).
- Global `--store <name>` flag (clap `global = true`) — override the active store
  for one command.
- Global `-C, --cwd <path>` flag (clap `global = true`, Git parity) — override the
  **operating directory** for one command (where `add`/`urn` scan and what keys are
  relative to), without changing the persisted content root.
- Every store-scoped verb (`add`, `commit`, `status`, `log`, `diff`, `checkout`,
  `cat`, `unstage`, `staged`, `urn`, `dir`, `remote`, `clone`, `push`, `pull`)
  operates on the resolved target store; `stores`/`use` operate on the workspace.

### 2.5 `CliContext` refactor

Today `CliContext.dig_dir` is the single store directory. Introduce:

- `workspace_dir: PathBuf` — the `.dig/` directory (discovered by walking up from
  CWD, like Git; or `<cwd>/.dig` for `init`).
- `store_dir(name) -> PathBuf` = `workspace_dir/stores/<name>`.
- Resolution helpers: `resolve_workspace(explicit_dig_dir)`, `load_workspace()`
  (reads `workspace.toml`), `active_store_name(--store)`, and `store_context(name)`
  returning a value whose `dig_dir` is the store dir — so the existing `store_ops`
  functions keep working unchanged against `dig_dir = .dig/stores/<name>/`.

The Phase-1 directory discovery (`resolve`/`resolve_init`) is updated: discovery
finds the `.dig/` workspace; the per-command "store dir" is then the selected
store's subdir. `--dig-dir` continues to override the workspace location.

### 2.6 Migration from the legacy single-store layout

A pre-existing `.dig/config.toml` (old single-store layout, no `.dig/stores/`) is
auto-migrated on first command: move the legacy store's files into
`.dig/stores/default/`, write `workspace.toml` with `active = "default"` and the
registry entry. Best-effort and idempotent; logged via the `Ui`. (Pre-1.0; this
keeps existing throwaway stores usable without a manual re-init.)

### 2.7 Workspace discovery (bubbles up, Git-style)

Every command finds the `.dig/` workspace by walking up from the CWD, so you can
run `digstore` from any subdirectory of the project. (`init` is the exception: it
creates `.dig/` in the CWD.) This is independent of the operating directory (§2.8):
discovery answers "which workspace/store", the operating directory answers "which
files".

### 2.8 Operating directory (content root) — what `add`/`urn` capture

Digstore captures **build output**, not repo files, so the directory a store
captures is a first-class, configurable notion distinct from where `.dig/` lives.

- **Content root** — each store may record a `content_root` in its `config.toml`,
  stored relative to the project root (the `.dig` parent) for portability. Unset by
  default; set it to a build dir with `init --dir dist` or `digstore dir dist`.
- **Operating directory** (the directory a single command scans) is resolved by:
  1. `-C/--cwd <path>` global flag, if given (one-shot override); else
  2. the selected store's persisted `content_root`, if set; else
  3. the **current working directory** (so `cd dist && digstore add -A` just works —
     consistent with capturing build output, never the whole repo by default).
- **`add -A` / `add .`** walk the operating directory's subtree (honoring
  `.digignore`/`.gitignore`, always skipping `.dig/`). Explicit paths/globs are
  resolved relative to the operating directory.
- **Resource keys are relative to the operating directory (the content root)**, not
  the CWD and not the project root. So with `content_root = dist`, a file at
  `<project>/dist/css/app.css` gets key `css/app.css` and a stable URN, regardless
  of which subdirectory you ran `add` from. (`walk::key_for` is computed against the
  resolved operating directory.)
- A path that escapes the operating directory (e.g. `../other`) is rejected — keys
  must stay within the content root so URNs are well-formed.

## 3. Per-store 100 MB cap

- `StoreConfig.max_size` defaults to **100_000_000** for new stores.
- `add` (the `add_files` op) computes `already_staged_bytes + incoming_bytes`
  (incoming = the resolved, deduped, not-byte-identical files) and, if it would
  exceed the store's `max_size`, **stages nothing** and returns
  `CliError::InvalidArgument` with:
  `staging would reach <X> MB, over the <name> store's 100 MB limit; stage fewer files or create another store (digstore init <name2>)`.
- The check is on staged content size (plaintext bytes), measured in decimal MB
  for the message. Enforced before any file is appended (atomic: all-or-nothing).
- `commit` keeps a defensive check too (a store whose generation content would
  exceed the cap is rejected), but the primary gate is at `add`.

### 3.1 Surfacing capacity everywhere (clear, always)

Store capacity is a first-class, always-visible part of the UX. Define, for the
selected store:

- **staged** = total plaintext bytes currently in the staging area (what the next
  `commit` will contain, which the 100 MB cap governs).
- **limit** = the store's `max_size` (100 MB).
- **free** = `max(0, limit − staged)`.

A small, reusable `Ui` capacity line renders this consistently — e.g.
`47.2 MB staged · 52.8 MB free of 100 MB  [#########·········]` (a compact usage
bar when color/TTY; plain `47.2 MB / 100 MB (52.8 MB free)` otherwise). It appears:

- in the `status` header (alongside the generation/root),
- as the footer of `staged` (which already lists per-file sizes + total),
- after every `add` (the new staged total + remaining headroom, and the file
  sizes just staged),
- in `stores` (a `used/limit` column per store),
- in the `add` over-limit error (how much over, and the free amount).

`digstore staged` is the dedicated "inspect staged space" view: each entry's size,
the total, and the remaining headroom. All of these honor `--json` (numbers in
bytes) and `--quiet` (suppress the decorative bar, keep the numbers in `status`).

## 4. Memory ceiling raise (compiler + host + guest)

The store content lives in the module's linear memory (injected data segment), so
the module memory must fit `DIGS_DATA_OFFSET + data_section_len + guest heap`. A
100 MB content store yields a data section of ~100 MB plus AEAD/merkle/filler
overhead; the guest also needs working heap above it.

### 4.1 Constants

- Define `MAX_STORE_BYTES = 100_000_000` (the content cap).
- Define a module memory **hard ceiling** `MAX_MEMORY_PAGES` large enough for the
  worst case: `ceil((DIGS_DATA_OFFSET + MAX_STORE_BYTES + overhead + heap_reserve)
  / 64KiB)`. Use **2048 pages = 128 MiB** as the hard cap (covers 100 MB content +
  ~1 MiB offset + AEAD/merkle/filler overhead + a heap reserve, with headroom).
  This replaces the current `256`.
- Host `MAX_MEMORY_BYTES` = `2048 * 64 KiB` (128 MiB), and remains the default of
  `ExecutionLimits.memory_bytes_max` (operator-configurable — the real DoS bound
  for untrusted modules, §18.2).

### 4.2 Compiler (`digstore-compiler`)

- `inject.rs`: the emitted module declares `minimum = needed_pages` (fit the data)
  and `maximum = MAX_MEMORY_PAGES` (the hard ceiling). The error
  `data section needs N pages but ceiling is M` fires only when
  `needed_pages > MAX_MEMORY_PAGES` (i.e. data alone exceeds 128 MiB — which the
  100 MB stage cap prevents, but keep the guard).
- `template.rs`: `MAX_MEMORY_PAGES = 2048`; `assert_memory_ceiling` requires
  `max == MAX_MEMORY_PAGES`. `load_template`'s tolerance check uses the new value.
- Golden fixture `golden_data_section.hex` / memory-ceiling tests updated to 2048.

### 4.3 Guest dynamic heap (`digstore-guest/src/allocator.rs`)

The heap must start **above** the actual data section, not at a fixed 8 MiB.

- The injected `DIGS` blob at `DIGS_DATA_OFFSET` is self-describing (magic,
  version, `u32` section count, offset table of `(id u16, off u32, len u32)`,
  then bodies). The **total blob length** = `max(off+len)` over the offset-table
  rows (equivalently the end of the last/Filler section).
- The bump allocator lazily computes `HEAP_BASE` on first allocation:
  `heap_base = align_up(DIGS_DATA_OFFSET + total_blob_len, 64KiB)` by reading the
  header from linear memory at `DIGS_DATA_OFFSET`. If the magic is absent (e.g.
  the bare template / native tests), fall back to the current fixed `8 MiB`.
- `HEAP_END` is no longer a fixed 16 MiB; allocation is bounded only by
  `memory.grow` success (which the host's `StoreLimits.memory_bytes_max` caps).
  An allocation that cannot grow memory returns null (OOM), as today.
- Contract D2 (heap never overlaps the data section) is now satisfied **for any
  data-section size**, not just <7 MiB.
- The `#[global_allocator]` static keeps `const fn new()`; the dynamic base is
  resolved on first `bump` (a `0` sentinel in `next` means "uninitialized → read
  header"). Native (test) builds keep the fixed base + system allocator.

> This is the highest-risk change: the secretless self-serving path
> (`content_proof`, `serve_flow`, `e2e_guest`, dighost) must still serve
> multi-chunk and now multi-MB resources without corrupting the pool. TDD against
> those suites; add a test that serves a resource whose data section exceeds the
> old 8 MiB heap base.

### 4.4 Spec/docs

Whitepaper §4.4 (workspace + named stores), §5.1 (memory: data section up to the
100 MB store cap; module memory ceiling 128 MiB; dynamic heap above the data),
§18.2 (host outer memory limit = 128 MiB default, operator-configurable), and a
new short "Multiple stores per project" subsection. Re-render the PDF.
`SECURITY.md`: note the per-store 100 MB cap and the raised, configurable host
memory bound.

## 5. Staging management

- `digstore unstage` — clear the active/selected store's staging area
  (`StagingArea::clear()`), report how many entries were dropped; `--json`:
  `{cleared: N}`. Optional `[paths...]` later (Phase: unstage-all now).
- `digstore staged` — list staged entries: `key` + human size, plus a total and
  the remaining headroom to the 100 MB cap; `--json`: `{staged: [{key, size}],
  total_bytes, limit_bytes}`. (This overlaps `status`'s staged group but is a
  dedicated, scriptable view.)

## 6. URN manifest on commit + URN preview

### 6.1 Local URN manifest (generated by `commit`)

Each `commit` (re)generates a local manifest of the URNs that address the new
generation's resources, so a publisher knows exactly what to share. Written to
`.dig/stores/<name>/urns.json` (overwritten each commit), plus a human-readable
`urns.txt`:

```json
{
  "store_id": "ab12…",
  "store": "docs",
  "root": "ef56…",
  "generation": 3,
  "resources": [
    { "key": "readme.md", "urn": "urn:dig:chia:ab12…:ef56…/readme.md",
      "retrieval_key": "…sha256(canonical_urn)…", "size": 1234 }
  ]
}
```

- The manifest is **local only** (not embedded in the module, not pushed) — it is
  the publisher's index of shareable URNs. It is regenerated from the committed
  generation's key table, so it always matches the latest root.
- `urns.txt` is one `key\turn` line per resource for easy copy/paste.
- The URN uses the root-pinned form (`…:<root>/<key>`); the manifest header notes
  that dropping `:<root>` addresses the current generation (§7.3).
- The manifest lives inside `.dig/` (gitignored).

### 6.2 `digstore urn` — preview URNs for project files

```
digstore urn [PATHS]...        # preview URNs for files (default: CWD subtree)
```

- Computes, without committing, the URN each given path *would* have:
  resource key = content-root-relative path; `urn:dig:chia:<storeID>[:<root>]/<key>`
  and its `retrieval_key = SHA-256(canonical_urn)`.
- Path resolution mirrors `add` (§2.8): files/dirs/globs relative to the operating
  directory; `-A`/`.` = its subtree; keys content-root-relative; `-C/--cwd`
  overrides for the one command.
- By default emits the **rootless** (current-generation) URN form; `--root <hex>`
  pins a specific generation. `--json` emits `[{path, key, urn, retrieval_key}]`.
- Works whether or not the file is staged/committed (it is a pure derivation from
  the path + the selected store's id), so it is useful for "what URN will this
  file have?" previews.

## 7. Testing

- **Unit:** store-name validation; selection precedence (`--store` > active >
  single > error); workspace.toml round-trip; the DIGS-blob total-length parser
  used by the allocator; dynamic `heap_base` computation (given a header, base is
  above the blob; absent header → fixed fallback); 100 MB cap arithmetic.
- **Integration (`assert_cmd`):** `init a` + `init b` create two stores;
  `stores` lists both with `*` on active; `use b` switches; `--store a add …`
  targets a regardless of active; per-store isolation (staging/log independent);
  `add` over 100 MB errors and stages nothing; `unstage` empties staging;
  `staged` lists entries + total; a ~20–30 MB store (over the old 16 MiB limit)
  `commit`s and `cat`s back correctly (ceiling + dynamic heap); legacy
  single-store `.dig/` is migrated to `stores/default` and remains usable.
- **Guest serve:** a resource whose embedded data section exceeds 8 MiB serves
  fully (no pool corruption) through the host runtime / dighost path.
- **Content root:** `init site --dir dist` records `content_root = dist`; `add -A`
  run from anywhere in the project captures `dist/`'s subtree with keys relative to
  `dist/` (a file at `dist/css/app.css` → key `css/app.css`); `digstore dir` prints
  it and `digstore dir build` updates it; `-C/--cwd other` overrides for one command
  without persisting; a path escaping the content root (`../x`) is rejected.
- **URN manifest + preview:** `commit` writes `urns.json`/`urns.txt` whose entries
  match the generation's resource keys with correct URNs/retrieval-keys; `digstore
  urn css/app.css` previews the right key/URN; the URN is identical whether `add`/
  `urn` was run from the content root or a subdirectory of it.
- **Full gate:** `cargo fmt`, `clippy -D warnings` (pinned 1.94.1),
  `cargo test --workspace`, `cargo deny check advisories bans sources`.

## 8. Out of scope

- Content sharding / a single logical resource spanning stores (explicitly
  rejected: over-cap content errors instead).
- Cross-store URN references.
- Remote/workspace-level multi-store push/pull orchestration (each store still
  pushes/pulls individually by its selected name → its store id/URL).
- Per-path `unstage <path>` (only clear-all in this pass).
