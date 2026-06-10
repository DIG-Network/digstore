# Bundled binaries (staging dir)

The installer ships the prebuilt `digstore` CLI inside the package. Tauri bundles
everything in this directory (`resources/bin/*`) into the app under `bin/`.

## What gets staged here

| File | Source | Notes |
|---|---|---|
| `digstore.exe` | `target/release/digstore.exe` | Windows binary (staged for this build). |
| `digstore.exe.sha256` | generated | SHA-256 digest checked by the gated verify step. |
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

## Signature verify

The install pipeline (`src-tauri/src/install.rs`, phase 2) recomputes the SHA-256
over the bundled binary and compares it to the `.sha256` sidecar. This is the
genuine, offline, **gated** integrity check (no network on first install). A
production release additionally signs this digest with a BLS key (see the spec's
"Verified package signature (BLS · 96 bytes)"); wiring that detached-signature
check is the one remaining TODO for full signature verification — the digest gate
is real today.
