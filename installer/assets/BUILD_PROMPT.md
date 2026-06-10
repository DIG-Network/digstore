# Build Prompt — DigStore Installer

Paste the text below to an LLM coding agent (e.g. Claude Code) that has this
`design_handoff_digstore_installer/` folder available in its workspace.

---

You are building a **real, cross-platform desktop installer** for **DigStore** — the
content-addressable WASM store format and CLI by DIG Network (`digstore`). A complete
design + spec package is in this folder. **Read it before writing code:**

- `README.md` — the authoritative spec. Window/rail/footer layout, all 5 wizard steps
  (exact copy, sizes, colors, type), the navigation state machine, design tokens, the
  **real install pipeline** to implement, and per-OS details. Build to this.
- `design/DigStore Installer.html` + `design/installer/installer-app.jsx` — a runnable,
  hi-fidelity HTML/React prototype. This is the visual + motion ground truth: match it.
  Serve `design/` statically and open the HTML to see it. The install in the prototype
  is **simulated** (a timed animation) — you must replace it with real work.
- `design/installer/tweaks-panel.jsx` — a prototype-only theming panel. Ignore for the
  shipped app (optionally fold into Settings).
- `brand/` — DIG Network token CSS (`colors_and_type.css` → `fonts.css`), bundled
  Space Grotesk fonts, and logos. Use these tokens verbatim; don't invent colors.
- `product/digstore-spec.txt` — the DigStore format spec (store module, ABI, URN
  system, encryption, attestation, execution proofs, CLI verbs). Use it so installer
  copy and the real install steps stay technically accurate.

## What to build

1. **Choose the stack.** Recommended: **Tauri** (Rust backend + web frontend) — it lets
   you reuse the prototype's HTML/CSS almost directly and matches DigStore's own Rust
   stack. Electron is acceptable; go fully native (SwiftUI / WiX-MSIX / Qt) only if
   required, treating the HTML as a visual spec. State your choice and why.

2. **Recreate the UI at high fidelity.** A frameless, rounded, centered window with the
   cosmic dark theme: navy `#101132` surface, purple→magenta gradient accents
   (`#5800D6`→`#FF00DE`), Space Grotesk / Open Sans / Space Mono type, the persistent
   left brand rail with the glow "D" + step list, and the footer action bar. Reproduce
   the five steps exactly as specified in `README.md`:
   **Welcome → License → Choose Components → Installing → Done.**
   Match spacing, radii, the gradient/glow treatments, and the `translateY` step-in
   transition. Keep content visible even if animations don't run (no opacity-0 base).

3. **Wire the real install pipeline** (replace the simulated progress). Drive the
   progress %, the "now writing" filename, and the streaming terminal log from actual
   work performed in the backend process:
   resolve release for the user's OS/arch → **download + verify package signature
   (a real, gated step)** → unpack the `digstore` CLI and host runtime to the chosen
   location → install selected components (shell completions, example store) → if
   selected, add `digstore` to PATH (symlink / PATH edit, requesting elevation only for
   the steps that need it) → verify (`digstore --version`) → report success.
   Implement an **error state** (the prototype has none): tint the progress fill red
   `#FF5C8A`, append the error to the log, and swap the CTA for **Retry** + **View log**.

4. **Honor per-OS behavior.** Default location macOS/Linux `/usr/local/digstore`
   (PATH symlink in `/usr/local/bin`); Windows `%LOCALAPPDATA%\Programs\DigStore`
   (add to user PATH). "Change…" opens a native folder picker. On the Done step,
   "Launch Terminal" opens the OS terminal and "Open Documentation" opens the docs URL.
   Make the title bar draggable; the window controls switch per the `chrome` setting
   (frameless / mac / win) as in the spec.

5. **State + persistence.** Model `step`, `agreed`, `selected` components, `installPath`,
   `pct`, `logLines`, `nowFile`, `copied`, and `error` as described in README →
   *State Management*. Gate Continue on license agreement (step 1) and install
   completion (step 3). Persist `step` but never resume mid-install.

## Constraints & done criteria

- Pixel-match the prototype's look; use only DIG brand tokens/assets from `brand/`.
- The DIG **mascot is not used** in this tool (developer tooling, not the consumer app).
- The HTML files are **references**, not code to ship verbatim — port them into a real,
  bundled app with proper dependency management.
- **Done =** a packaged installer per platform that, when run, presents the five-step
  flow and actually installs a working `digstore` (verified by `digstore --version` and
  the three Done-step commands succeeding: `digstore init` / `add` / `commit`), with the
  signature-verify and error states wired, matching the design.

If anything in the spec is ambiguous, prefer the prototype's visible behavior, then
`product/digstore-spec.txt` for technical accuracy.
