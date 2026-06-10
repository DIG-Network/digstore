# DigStore Installer (Tauri 2)

A real, cross-platform desktop installer for the DigStore CLI + host runtime,
built to `installer/assets/README.md` (the authoritative spec) and the hi-fi
prototype in `installer/assets/design/`.

- **Stack:** Tauri 2 — Rust backend (`src-tauri/`) + bundled Vite + React 18
  frontend (`src/`). The frontend ports the prototype's HTML/CSS/JSX at high
  fidelity using only the DIG brand tokens (`installer/assets/brand/`).
- **Delivery:** bundle binaries. The installer ships the prebuilt `digstore`
  binary inside the package (`src-tauri/resources/bin/`). Install =
  verify-checksum (gated, offline SHA-256) → unpack → install components →
  add to PATH → verify `digstore --version`. No network download on first install.

## Layout

```
installer/app/
├─ index.html                 # Vite entry
├─ vite.config.js             # Tauri-tuned Vite config (port 1420, relative base)
├─ package.json
├─ scripts/stage-binary.mjs   # copies target/release/digstore[.exe] → resources/bin + sha256
├─ src/                       # React frontend (the wizard)
│  ├─ main.jsx
│  ├─ App.jsx                 # state machine + rail + footer + real install wiring
│  ├─ bridge.js               # the ONE seam to the host (Tauri IPC ↔ browser sim)
│  ├─ TitleBar.jsx            # frameless / mac / win chrome + real window controls
│  ├─ icons.jsx  data.jsx     # icon set + copy/data arrays (lifted from prototype)
│  ├─ styles.css              # prototype CSS ported verbatim + new error-state CSS
│  ├─ steps/                  # Welcome / License / Components / Installing / Finish
│  └─ assets/                 # bundled Space Grotesk fonts, glow-D, nebula, favicon
└─ src-tauri/                 # Rust backend (its own workspace; isolated from crates/)
   ├─ Cargo.toml              # empty [workspace] → never compiles the digstore crates
   ├─ tauri.conf.json         # frameless transparent window; nsis/msi/dmg/deb/appimage
   ├─ capabilities/default.json
   ├─ resources/bin/          # staged digstore binary (+ .sha256) — bundled into the package
   └─ src/
      ├─ main.rs lib.rs       # Tauri commands: installer_meta, default_install_path,
      │                       #   run_install, cancel_install, launch_terminal
      └─ install.rs           # the real 6-phase install pipeline
```

## Run it

Dev (hot-reload frontend + Rust):

```
cd installer/app
npm install
node scripts/stage-binary.mjs        # stage the real binary (builds nothing)
npm run tauri dev
```

If `target/release/digstore[.exe]` doesn't exist yet, build only the CLI
(does NOT build the whole workspace):

```
cargo build -p digstore-guest --target wasm32-unknown-unknown --release
cargo build -p digstore-cli --release
```

Package the installer (NSIS .exe + MSI on Windows):

```
cd installer/app
node scripts/stage-binary.mjs
npm run tauri build
# → src-tauri/target/release/bundle/{nsis,msi}/
```

## Pixel-matched to the prototype

All five steps, the 332px brand rail (glow-D, wordmark, version pill, 5-step list
with upcoming/active/done states), the 46px draggable title bar with
frameless/mac/win chrome, the footer action bar with progress dots, the
`translateY(9px→0)` `.42s` step-in (content never opacity-0), `prefers-reduced-motion`,
the license box, component rows with live totals, the install terminal with
colored log spans + blinking caret, and the success seal + copyable next-steps.
The **error state** (red fill, error banner, Retry + View log) is new, designed
from the existing tokens (`--error`).

## Real pipeline status — wired vs stubbed

| Phase | Status |
|---|---|
| Resolve OS/arch target | wired (`std::env::consts`) |
| **Checksum verify (gated, offline)** | wired as a real SHA-256 manifest check against the bundled binary — a checksum (integrity), not cryptographic provenance, since the manifest ships next to the binary. **TODO:** add the BLS detached-signature check over that digest for real provenance / full parity with the spec's "BLS · 96 bytes". |
| Unpack CLI | wired — copies the real bundled `digstore[.exe]` into `<dir>/bin` (chmod 755 on unix). |
| Host runtime | partial — the host runtime ships inside the CLI today; the installer writes a marker into `<dir>/lib`. **TODO:** copy a standalone `dig_host` artifact when one exists. |
| Shell completions | partial — writes placeholder scripts. **TODO:** `digstore completions <shell>` once the CLI emits them. |
| Example store | partial — writes a README placeholder. **TODO:** bundle a real sample `urn:dig` store. |
| Add to PATH | wired — `setx PATH` (HKCU, no elevation) on Windows; symlink into `/usr/local/bin` on unix (needs elevation). |
| Verify `digstore --version` | wired — runs the unpacked binary; failure → error state. |

The bundled-binary source for dev/CI is `target/release/digstore[.exe]`, staged
by `scripts/stage-binary.mjs`. CI (`.github/workflows/release.yml`) builds the
CLI and stages it before `tauri build`.

## Per-OS coverage

- **Windows** (built + verified here): `%LOCALAPPDATA%\Programs\DigStore`, user
  PATH via `setx`, `win` window chrome, NSIS + MSI bundles.
- **macOS / Linux**: code paths implemented (`/usr/local/digstore`, `/usr/local/bin`
  symlink, `mac` chrome, terminal launch via `open`/`x-terminal-emulator`). Bundle
  targets (`dmg`/`app`, `deb`/`appimage`) are in `tauri.conf.json` and one
  commented matrix entry in the release workflow away from shipping. Not built on
  this machine (Windows only).
