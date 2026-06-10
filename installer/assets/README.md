# Handoff: DigStore Installer (DIG Network)

## Overview

This package specifies a **cross-platform desktop installer** for **DigStore** — the
content-addressable WASM store format and CLI by DIG Network. The installer walks a
user from a welcome screen through license acceptance, component selection, a live
install with a streaming log, and a completion screen with next-step commands.

The goal of this handoff is for a developer (or an LLM agent such as Claude Code) to
**build a real, working installer** whose UI matches the supplied design and whose
behavior matches the functional spec in this document. The design is hi-fidelity:
final colors, type, spacing, copy and motion are all specified.

> **DigStore in one line:** every store compiles to a single portable `.wasm` that
> embeds its own (encrypted, content-addressed) content and the logic that serves it.
> The developer workflow is Git-shaped (`init`, `add`, `commit`, `log`, `clone`). The
> CLI binary is `digstore`. Full product spec: `product/digstore-spec.txt`.

---

## About the Design Files

The files in `design/` are a **design reference created in HTML/CSS + React (via
Babel in the browser)**. They are a faithful, interactive prototype of the intended
look and behaviour — **not production code to ship verbatim**.

Your task is to **recreate this design in the appropriate native/desktop installer
technology**, using that environment's idioms. Recommended targets (pick to fit the
project / team):

- **Tauri** (Rust + web frontend) — the web frontend can reuse the HTML/CSS here almost
  directly; Rust handles the actual install work. **Best fit** given the design is
  already web tech and DigStore itself is Rust.
- **Electron** (Node + web frontend) — also lets you reuse the markup/CSS; heavier.
- **Native** — Swift/SwiftUI (macOS `.app`/`.pkg`), WiX/MSIX (Windows), or a Qt app —
  if a fully native feel is required. In this case treat the HTML as a visual spec only.

If reusing the web frontend, port the React-in-Babel prototype to a proper bundled
React/Vite (or Svelte/Vue) app and replace the **simulated** install (a timed
animation) with **real** install steps wired to the host process (see *Interactions &
Behavior → Real install pipeline*).

Run the prototype as-is by serving `design/` over any static server and opening
`DigStore Installer.html` (it needs the sibling `installer/` folder for scripts,
fonts and images).

## Fidelity

**High-fidelity.** Recreate the UI pixel-accurately using the exact tokens in
*Design Tokens*. Match the cosmic dark theme, the purple→magenta gradient accents,
Space Grotesk / Open Sans / Space Mono type, the frameless window chrome, and the
fade/slide step transition. Layout should be fixed-size (it's a window, not a
responsive web page) but tolerate the window being resized down gracefully.

---

## Window & Global Layout

A **frameless, rounded application window**, centered on the desktop.

- Window: `width: min(1080px, 94vw)`, `height: min(720px, 90vh)`, `border-radius: 18px`
  (density-dependent; see tokens), `border: 1px solid rgba(255,255,255,.08)`, and a
  deep drop shadow with a faint violet bloom:
  `box-shadow: 0 50px 130px rgba(0,0,0,.65), inset 0 0 0 1px rgba(255,255,255,.03), 0 0 80px rgba(88,0,214,.12)`.
- Window background: `#101132` (brand navy).
- The page behind the window is the "desktop": a near-black `#06061a` with two large
  radial glows (violet top-left, magenta bottom-right). Body padding `36px`.

Three horizontal bands inside the window, top to bottom:

1. **Title bar** — height `46px`, full width.
2. **Body** — fills remaining height; a **left brand rail** (fixed `332px`) + a
   **right content column** (flex-1) side by side.
3. The content column itself splits into a **scrollable pane** (flex-1) and a
   **footer action bar** (auto height).

```
┌───────────────────────────────────────────────────────────┐
│ ● DigStore Installer                              ○ ○ ●     │  title bar (46px)
├──────────────────┬────────────────────────────────────────┤
│  [glow D]         │  EYEBROW                                │
│  DigStore         │  Step Title (gradient word)            │
│  tagline          │  lead paragraph…                        │  pane (scrolls)
│  v1.0.0 pill      │  …step content…                         │
│                   │                                         │
│  ① Welcome ✓      │                                         │
│  ② License ✓      │                                         │
│  ③ Components ●   ├────────────────────────────────────────┤
│  ④ Install        │ ●●●○○         [ Back ]   [ Primary CTA ] │  footer
│  ⑤ Done           │                                         │
│  · PoS L2 on Chia │                                         │
└──────────────────┴────────────────────────────────────────┘
   332px rail                  flex-1 content
```

---

## Title Bar

- Height `46px`, bottom border `1px solid rgba(255,255,255,.08)`, subtle top sheen
  `linear-gradient(180deg, rgba(255,255,255,.04), transparent)`.
- **Title** (center for mac chrome, left otherwise): a `15×15` rounded square with the
  brand gradient fill containing a tiny white horizontal-bar glyph, then the text
  **"DigStore Installer"** in Space Grotesk 600, `13.5px`, color `#C5C1E0`.
- **Window controls** — switchable by the `chrome` tweak:
  - `frameless` (default): three `12px` dots on the right; first two
    `rgba(255,255,255,.16)`, the close (last) filled with the brand gradient.
  - `mac`: three `12px` traffic-light dots on the **left** (`#FF5F57`, `#FEBC2E`,
    `#28C840`), title centered.
  - `win`: right-aligned minimize / maximize / close glyph buttons (`40px` wide,
    full bar height); close hover background `#e81123`.
- The bar reads as draggable (in Tauri/Electron mark it `data-tauri-drag-region` /
  `-webkit-app-region: drag`, with the controls `no-drag`).

---

## Left Brand Rail (persistent across all steps)

Fixed `332px`, padding `30px 28px 24px`, right border `1px solid rgba(255,255,255,.08)`.
Background is a layered navy gradient:
`linear-gradient(180deg, rgba(10,8,30,.2), rgba(10,8,30,.86)), linear-gradient(160deg,#1a0f4a 0%,#11103a 55%,#0c0a28 100%)`.

- **Nebula** (decorative, toggled by `nebula` tweak): the image
  `assets/galaxy-background.webp` pinned across the top ~62% of the rail,
  `opacity: .6`, `mix-blend-mode: screen`, masked to fade out toward the bottom
  (`linear-gradient(180deg,#000 40%,transparent)`).
- **Glow D**: `assets/D-glow-logo.svg` at `74×74`, with a radial magenta/violet glow
  behind it (`::before` radial gradient) and `filter: drop-shadow(0 0 14px rgba(255,61,245,.55))`.
- **Wordmark**: "DigStore" — Space Grotesk 700, `27px`, letter-spacing `-.02em`.
- **Tagline**: "The content-addressable WASM store format, by DIG Network." —
  `13px`, color `#9A95C0`, max-width `230px`.
- **Version pill**: pill with a glowing green dot (`#38E1B0`) + Space Mono `11.5px`
  text "v1.0.0 · compiler 1.0.0", bg `rgba(255,255,255,.05)`, border
  `1px solid rgba(138,110,255,.18)`.
- **Step list** (pushed to bottom with `margin-top:auto`): five rows — Welcome,
  License, Components, Install, Done. Each row: a `25px` index circle + label
  (Space Grotesk 500, `14px`).
  - **Upcoming**: muted `#9A95C0`, circle is an outlined ring with the number.
  - **Active** (current step): row bg `rgba(255,255,255,.05)`, border
    `1px solid rgba(138,110,255,.18)`, text white, circle ring glows magenta
    (`box-shadow: 0 0 0 4px rgba(255,0,222,.12)`).
  - **Done** (past step): text `#C5C1E0`, circle filled with brand gradient showing a
    white check. Past rows are **clickable** to navigate back (except during Install).
- **Rail footer**: a tiny ringed-circle icon + "A Proof-of-Stake Layer 2 on Chia",
  `11px`, color `#6E6A99`, separated by a top hairline.

---

## Screens / Views

There are **5 steps**, indexed 0–4. The pane content fades/slides in (`translateY(9px)
→ 0`, `.42s ease`) whenever the step changes — keep content at full opacity so it is
**never** hidden if the animation is interrupted. Common pane header pattern:
`EYEBROW` (Space Grotesk 600, `12px`, uppercase, letter-spacing `.18em`, color
`#FF00DE`) → `h2` title (Space Grotesk 700, `34px`, `-.02em`) with one word wrapped in
the gradient-text helper → `lead` paragraph (`15.5px`, `#C5C1E0`, max-width `560px`).
Pane padding `44px 56px` (density-dependent).

### Step 0 — Welcome
- **Purpose:** orient the user; primary "Install" entry point.
- Eyebrow: "DigStore CLI · Host Runtime". Title: **Install `DigStore`** (gradient on
  "DigStore"). Lead: "The content-addressable WASM store format. Your content and the
  logic that serves it compile into one portable, encrypted, self-defending
  executable."
- **Feature list** (3 rows, gap `14px`, max-width `580px`). Each: a `40px` gradient
  icon tile (`border-radius:11px`, `linear-gradient(160deg,rgba(120,30,235,.32),rgba(255,0,222,.16))`,
  border `1px solid rgba(138,110,255,.18)`, `20px` stroked icon `#E4C7FF`), a heading
  (Space Grotesk 600, `15.5px`) and a `13.5px` `#9A95C0` paragraph:
  1. **A Git-shaped workflow** (git-node icon) — "init, add, commit, log, diff,
     checkout, clone — the verbs you already know. Chunking, encryption and WASM
     compilation stay under the surface."
  2. **Encrypted at rest, by URN** (lock icon) — "Every URN is a key. Content is
     chunked, SHA-256 addressed, and sealed with an AES-256-GCM key derived from the
     URN itself."
  3. **Provable & secretless** (shield-check icon) — "Each store compiles to one
     portable .wasm that defends itself — merkle proofs, host attestation, and
     zero-knowledge proofs of execution. No embedded secret to extract."
- **Meta chips** (pill row, Space Mono `12px`, bg `rgba(255,255,255,.04)`, border
  `1px solid rgba(255,255,255,.08)`): `version 1.0.0`, `install size ~46 MB`,
  `platforms macOS · Linux · Windows`, `license Apache-2.0`.
- **Primary CTA:** "Install DigStore". No Back button.

### Step 1 — License Agreement
- **Purpose:** present terms; gate continuation on agreement.
- Eyebrow "Step 02 — Terms". Title "License Agreement". Lead "Review the terms below.
  DigStore is open source under the Apache License 2.0."
- **License box:** `height: 288px`, scrollable, bg `#0A0A20`, border
  `1px solid rgba(255,255,255,.08)`, radius `14px`, Space Mono `12.5px`, line-height
  `1.85`, color `#C5C1E0`. Contains an Apache-2.0-style EULA whose clauses reflect the
  product: (1) Grant, (2) The module is the artifact, (3) URN as credential, (4)
  Provider blindness, (5) Warranty AS-IS, (6) Limitation of liability. Use the exact
  text in `design/installer/installer-app.jsx` (`License` component) — or ship the real
  Apache-2.0 text plus a short DigStore preamble.
- **Agree control:** a `22px` custom checkbox + label "I have read and agree to the
  DigStore License Agreement." Unchecked = outlined; checked = brand-gradient fill,
  white check, magenta glow. Clicking the row toggles it.
- **Footer:** Back (enabled) + **Continue** (disabled until the box is checked —
  rendered at `opacity:.4`, `cursor:not-allowed`).

### Step 2 — Choose Components
- **Purpose:** pick install location and optional components.
- Eyebrow "Step 03 — Setup". Title "Choose Components". Lead "Pick what to install and
  where. The CLI is required; everything else is optional."
- **Install location:** label "INSTALL LOCATION" then a row (max-width `620px`): a
  read-only path field (bg `#0A0A20`, Space Mono `13px`, folder icon, value
  `/usr/local/digstore`) + a ghost **"Change…"** button. (Wire Change… to a native
  folder picker in the real app; default per-OS path below.)
- **Component rows** (each max-width `620px`, padding `16px`, radius `14px`, border
  `1px solid rgba(255,255,255,.08)`, a `22px` checkbox on the left, name in Space
  Grotesk 600 `15px`, description `12.5px` `#9A95C0`, and a right-aligned size in Space
  Mono `12px` `#6E6A99`):
  | Component | Description | Size | Default |
  |---|---|---|---|
  | **DigStore CLI** | The `digstore` command — init, add, commit, log, clone. | 18.4 MB | required (locked on, shows a magenta "REQUIRED" pill instead of a size) |
  | **Host Runtime** | Sandboxed WASM host with attestation + session ABI. | 21.0 MB | on |
  | **Shell completions** | bash · zsh · fish tab-completion for `digstore`. | 0.3 MB | on |
  | **Add `digstore` to PATH** | Symlink `digstore` into `/usr/local/bin`. | — | on |
  | **Example store** | A sample `urn:dig` store to clone and explore. | 6.1 MB | off |
  Clicking a non-required row toggles its checkbox.
- **Totals chips:** "total download ~X MB" (sum of selected sizes) and "disk after
  install ~Y MB" (`total × 1.4`), recomputed live.
- **Footer:** Back + **Install**.

### Step 3 — Installing
- **Purpose:** run and visualize the install. **No Back; CTA disabled** until complete.
- Eyebrow "Step 04 — Installing". Title "Installing DigStore" → becomes "Install
  complete" at 100%.
- **Progress header:** large percent (Space Grotesk 700, `30px`) on the left; on the
  right a muted Space Mono "writing  <current file>" that becomes "done" at 100%.
- **Track + fill:** `10px` rounded track `rgba(255,255,255,.07)`; fill uses the brand
  gradient with a magenta glow (`box-shadow: 0 0 18px rgba(255,0,222,.5)`),
  `transition: width .25s ease`.
- **Terminal log:** `height: 230px`, scrollable, bg `#0A0A20`, Space Mono `12.5px`,
  auto-scrolls to bottom as lines append. A blinking magenta caret `▍` shows while
  in progress. Log lines (prototype timings shown for reference; in the real app emit
  these as each phase completes):
  ```
  $ digstore-setup --target /usr/local/digstore
  Resolving release v1.0.0 · compiler 1.0.0 · module format 1
  ✓ Verified package signature (BLS · 96 bytes)
  Unpacking DigStore CLI → /usr/local/digstore/bin
  Unpacking Host Runtime (64 KiB → 16 MiB memory bounds)
  Embedding trusted host keys dig-host-key-v1:…
  ✓ Content-defined chunking ready (16/64/256 KiB)
  Linking digstore → /usr/local/bin/digstore
  Installing shell completions bash · zsh · fish
  ✓ Verifying install · merkle root committed
  ✓ DigStore is ready.
  ```
  Highlight spans: `✓` and success text green `#38E1B0`; component/path accents
  magenta `#FF3DF5`; parenthetical/dim notes `#6E6A99`.
- **Footer:** CTA reads "Installing…" (disabled) then "Continue" at 100%.

### Step 4 — Done
- **Purpose:** confirm success and hand off next steps.
- **Success seal:** `108px` circle, centered, with a green radial bloom + a `2px`
  green ring + a `50px` green check.
- Title: **DigStore is `installed`** (gradient on "installed"). Centered lead: "The CLI
  and host runtime are ready. Initialize your first content-addressable store and
  commit a generation."
- **Recap chips** (centered): `version 1.0.0`, `location /usr/local/digstore`, and a
  green-dot "digstore on PATH".
- **Next steps** block (max-width `560px`, left-aligned): label "NEXT STEPS" then a
  terminal card (bg `#0A0A20`, Space Mono `13px`) with a **Copy** button (top-right)
  that copies all three commands:
  ```
  $ digstore init my-store     # create a store
  $ digstore add ./site        # stage content
  $ digstore commit -m "v1"    # compile a .wasm generation
  ```
  Copy button shows a check + "Copied" for ~1.6s after click.
- **Footer:** secondary "Open Documentation" + primary "Launch Terminal".

---

## Interactions & Behavior

**Navigation / state machine**
- `step` ∈ {0..4}. Footer **Primary** advances `step+1` when `canContinue` is true;
  **Back** goes `step-1` (hidden on steps 0, 3, 4). Past steps in the rail are
  clickable to jump back, **disabled while installing (step 3)**.
- `canContinue`: step 1 requires `agreed === true`; step 3 requires install complete
  (`pct >= 100`); all other steps always true.
- Primary label by step: 0 "Install DigStore", 2 "Install", 3 "Installing…" → "Continue",
  4 "Launch Terminal", else "Continue".
- Entering step 3 **starts the install** (see below). On step 4, "Launch Terminal"
  in the prototype loops back to step 0; in the real app it should launch a terminal
  (or close the installer).
- Persist `step` to `localStorage` so a reload restores position, **but never resume
  mid-install** — if the stored step is 3, fall back to 2.

**Animations**
- Step transition: `@keyframes` translateY `9px → 0` over `.42s ease`, re-triggered by
  keying the pane on `step`. **Do not animate opacity from 0** — keep content visible
  even if the animation never runs (prototype learned this the hard way). Respect
  `prefers-reduced-motion`.
- Progress fill width transitions `.25s ease`; blinking caret in the log.
- Primary button hover: `brightness(1.08)` + `translateY(-1px)`; disabled removes
  shadow/transform and drops to `opacity:.4`.

**Real install pipeline (replace the simulated animation)**
The prototype fakes the install with a `requestAnimationFrame` ramp + timed log lines.
In the real installer, drive `pct`, the "now writing" filename, and log lines from the
**actual** steps, ideally streamed from the host process (Tauri command / Electron IPC /
native task):
1. Resolve the target release for the user's OS/arch.
2. Download the package and **verify its signature** before unpacking (the product is
   security-first — surface the verification as a real, gated step, not decoration).
3. Unpack the `digstore` CLI and the host runtime into the chosen location.
4. Install selected components (shell completions; example store).
5. If "Add to PATH" is selected, create the symlink / update PATH (may require elevation
   — see below).
6. Verify the install (run `digstore --version`) and report success.
On any failure, show an **error state**: stop the progress fill (tint it red `#FF5C8A`),
append the error to the log, and replace the CTA with "Retry" + "View log". (The
prototype has no error state — design one consistent with the tokens.)

**Per-OS specifics to implement**
- Default install location: macOS/Linux `/usr/local/digstore` (PATH symlink in
  `/usr/local/bin`); Windows `%LOCALAPPDATA%\Programs\DigStore` (add to user PATH).
- Writing to `/usr/local` or modifying PATH typically needs **elevated privileges** —
  request elevation only for the steps that need it, and reflect a "Requesting
  permission…" state in the log.
- "Change…" opens the native folder picker. "Launch Terminal" / "Open Documentation"
  open the OS terminal and the docs URL respectively.

---

## State Management

| State | Type | Purpose |
|---|---|---|
| `step` | int 0–4 | current wizard step (persisted to localStorage; never resumes at 3) |
| `agreed` | bool | license acceptance; gates step 1 → 2 |
| `selected` | map of componentId→bool | optional component choices (CLI always on) |
| `installPath` | string | chosen install directory |
| `pct` | 0–100 | install progress |
| `logLines` | string[] | terminal output, appended as phases complete |
| `nowFile` | string | file currently being written (progress header) |
| `copied` | bool | transient "Copied" feedback on the Done step |
| `error` | object/null | (add for real app) failure info for the error state |

Triggers: Primary/Back/rail clicks mutate `step`; entering `step 3` kicks off the
install task which streams `pct`/`nowFile`/`logLines`; completion enables Continue.

---

## Design Tokens

Authoritative source: `brand/colors_and_type.css` (+ `brand/fonts.css`). Key values:

**Brand color**
- DIG Violet (primary) `#5800D6` · Magenta (secondary) `#FF00DE` · Pink glow `#FF3DF5`
- Signature gradient: `linear-gradient(115deg, #5800D6 0%, #FF00DE 100%)` (the
  installer exposes the two stops as `--accent-a` / `--accent-b`; the `accent` tweak
  swaps them).
- Gradient text: `linear-gradient(100deg, #B98CFF 0%, #FF6BEE 100%)` (clip to text).

**Surfaces (dark)**
- Void `#0A0A20` · Space/navy `#101132` · Panel `#16153A` · Card `#191248` ·
  Card-2 `#221a59` · desktop behind window `#06061a`.

**Borders** — subtle `rgba(255,255,255,.08)` · soft `rgba(138,110,255,.18)` · bright
`rgba(255,0,222,.55)`.

**Text** — primary `#FFFFFF` · body `#C5C1E0` · muted `#9A95C0` · faint `#6E6A99`.

**Status** — ok `#38E1B0` · warn `#E0A640` · error `#FF5C8A`.

**Type families** — display/UI **Space Grotesk** (400/500/600/700, bundled TTFs in
`brand/fonts/`), body **Open Sans**, mono **Space Mono** (both Google Fonts). Scale used
here: h2 `34px`/700, h1(rail) `27px`/700, feature head `15.5px`/600, lead `15.5px`,
body `13.5–14px`, eyebrow `12px` uppercase `.18em`, mono `12–13px`.

**Radii** — sm `8px` · md `14px` · lg `20px` · pill `999px` · window `14/18/22px`
(compact/regular/comfy).

**Elevation** — card `0 18px 50px rgba(0,0,0,.45)`; window shadow as in *Window Layout*.

**Tweaks (user-toggleable in the prototype; expose as installer themes/settings if
desired)** — `accent` (4 curated gradient pairs), `chrome` (frameless/mac/win window
controls), `nebula` (background imagery on/off), `density` (compact/regular/comfy →
swaps pane padding + window radius via CSS vars).

---

## Assets

All under `design/installer/assets/` (and originals in `brand/`):
- `D-glow-logo.svg` — glowing "D" sigil for dark backgrounds (rail mark). Glow is added
  in CSS; the SVG itself is the white-ish "D".
- `Wordmark-white.svg` — horizontal DIG wordmark (white) — available if you prefer it
  over the text "DigStore" lockup.
- `GRADIENT-D.png` — gradient "D" for light backgrounds (not used in the dark UI; for
  light installer skins).
- `galaxy-background.webp` — soft nebula used in the rail (semi-transparent, screen
  blend). `Background.webp`, `grid-background.webp` — alternate cosmic backdrops.
- `favicon.png` — app icon source.
- `fonts/space-grotesk-{400,500,600,700}.ttf` — bundle these with the app.

Icons in the UI (git-node, lock, shield-check, folder, check, copy, window-control
glyphs) are inline SVGs defined in `installer-app.jsx` (`Ic` object) — copy them or
substitute your icon set. Use brand colors; don't introduce new hues.

**Brand rules:** keep clear space around the wordmark ≈ the height of the "D"; never
recolor/stretch the logo; glow "D" on dark only (gradient/black "D" on light). The DIG
**mascot** is intentionally **not** used here — it's reserved for the consumer DIGHub
app, not the developer tooling.

---

## Files

In this bundle:
- `design/DigStore Installer.html` — entry point; loads React 18.3.1 + Babel + the two
  scripts below. Inline `<style>` holds all tokens, the window/rail/footer chrome, and
  every step's CSS.
- `design/installer/installer-app.jsx` — the whole wizard: `App` (state machine +
  tweaks), `TitleBar`, and the `Welcome` / `License` / `Components` / `Installing` /
  `Finish` step components, plus the `Ic` icon set and the data arrays (`COMPONENTS`,
  `INSTALL_LOG`, `FEATURES`). **Read this for exact copy and structure.**
- `design/installer/tweaks-panel.jsx` — the in-prototype Tweaks panel + controls
  (preview affordance only; not part of the shipped installer).
- `design/installer/assets/`, `design/installer/fonts/` — images + bundled fonts.
- `brand/colors_and_type.css`, `brand/fonts.css`, `brand/fonts/`, `brand/logos/` —
  the DIG Network brand token + asset source of truth.
- `product/digstore-spec.txt` — the DigStore format spec (store module, ABI, URN
  system, encryption, attestation, execution proofs, CLI). Use it to keep installer
  copy and the real install steps technically accurate.

A developer who was not in this conversation should be able to build the installer
from this README alone; open the prototype to see the motion and exact spacing.
