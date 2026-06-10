# Bundled binaries (staging dir)

The installer ships the prebuilt `digstore` CLI inside the package. Tauri bundles
everything in this directory (`resources/bin/*`) into the app under `bin/`.

## What gets staged here

| File | Source | Notes |
|---|---|---|
| `digstore.exe` | `target/release/digstore.exe` | Windows binary (staged for this build). |
| `digstore.exe.sha256` | generated | SHA-256 checksum checked by the gated verify step. |
| `digstore` | `target/release/digstore` | macOS/Linux binary (stage on those platforms). |
| `digstore.sha256` | generated | digest sidecar for the unix binary. |

## How it is staged

CI / local builds run the staging script before `tauri build`:

```
# from installer/app
node scripts/stage-binary.mjs          # copies the current-OS binary + writes its .sha256
```

or in CI the release workflow builds `digstore` with
`cargo build -p digstore-cli --release` and copies the artifact here.

## Checksum verify

The install pipeline (`src-tauri/src/install.rs`, phase 2) recomputes the SHA-256
over the bundled binary and compares it to the `.sha256` sidecar. This is a
**checksum (SHA-256)**, not cryptographic provenance: the manifest travels next
to the binary, so it proves integrity (no corruption/truncation), not authorship.
It is a genuine, offline, **gated** check (no network on first install) and aborts
the install before any unpack/exec.

Real cryptographic provenance — a **BLS detached signature** over this digest
(see the spec's "Verified package signature (BLS · 96 bytes)") — remains the
documented **TODO**. Wiring that detached-signature check is what would upgrade
this from a checksum to a verified signature; the checksum gate is real today.
