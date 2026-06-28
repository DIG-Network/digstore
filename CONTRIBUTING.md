# Contributing to digstore

Thanks for your interest in improving DigStore. This is a security-sensitive,
content-addressable store that compiles to a self-defending WebAssembly module —
please read this before opening a PR.

## Prerequisites

- [Rust](https://rustup.rs), pinned to **1.94.1** via `rust-toolchain.toml`
  (`rustup` picks it up automatically), with the `wasm32-unknown-unknown` target.
- The CLI's `build.rs` embeds the compiled guest WASM, so build the guest first:

  ```sh
  cargo build -p digstore-guest --target wasm32-unknown-unknown --release
  ```

## Build & test

```sh
# build the whole workspace
cargo build --workspace

# run the full test suite
cargo test --workspace
```

`.cargo/config.toml` sets `DIGSTORE_UNIFORM_BLOB_LEN=1048576` so tests don't each
compile a full ~128 MiB module — leave it in place for fast, green runs. The one
exception is the `#[ignore]`d near-cap serve test, which sets the full budget
explicitly; run it with `cargo test -p digstore-compiler --test large_data_section -- --include-ignored`.

## The gate (must pass before a PR is merged)

CI runs these on every PR (`.github/workflows/ci.yml`); run them locally first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- \
  -D warnings \
  -A clippy::default_constructed_unit_structs \
  -A clippy::field_reassign_with_default
cargo test --workspace --locked
cargo deny check advisories bans sources
```

## Commit conventions

- **Sign your commits.** The project uses SSH commit signing (`gpg.format=ssh`,
  `commit.gpgsign=true`). Make sure your signing key is registered with GitHub so
  commits show as **Verified**.
- Use clear, imperative commit subjects (e.g. `feat(cli): …`, `fix(compiler): …`,
  `docs: …`, `test: …`). Keep one logical change per commit where practical.
- **Do not add `Co-Authored-By` trailers.**

## Where things live

| Crate | Responsibility |
|---|---|
| `digstore-core` | Shared types, data-section format, URN, merkle, limits |
| `digstore-crypto` | AES-256-GCM-SIV chunk AEAD, BLS, HKDF key derivation |
| `digstore-chunker` | Content-defined chunking |
| `digstore-store` | On-disk store: config, staging, generations |
| `digstore-compiler` | Compiles a generation into the serving `.wasm` module |
| `digstore-guest` | `no_std` WASM guest: serves content, builds proofs |
| `digstore-host` | Sandboxed `wasmtime` host runtime + limits |
| `digstore-prover` | Execution-proof backends (mock + RISC0) |
| `digstore-remote` | HTTPS sync server/client |
| `digstore-cli` | The `digstore` binary (owns client-side crypto + UX) |

The desktop/universal installer now lives in its own repo,
[DIG-Network/dig-installer](https://github.com/DIG-Network/dig-installer); this
repo only builds and publishes the `digstore` binary that the installer consumes.

## Security

For anything security-relevant, read `SECURITY.md` first and **report
vulnerabilities privately** to the maintainer rather than opening a public issue.
The design of record is the whitepaper in `docs/whitepaper/`.

## Pull requests

1. Branch from `main`.
2. Make the gate green locally.
3. Open a PR with a clear description of the change and its rationale; reference
   any related issue. Keep the diff focused.
