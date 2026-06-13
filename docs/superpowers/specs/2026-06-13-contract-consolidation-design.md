# Cross-layer contract consolidation — master design

**Date:** 2026-06-13
**Status:** SP1 approved (autonomous), implementing
**Repos:** `digstore_wasm` (the `.dig` format owner) + `hub.dig.net` (blind creator/consumer)

## Problem

A `.dig` generation compiled by the old `0.5.6` producer became permanently
unreadable by the deployed verifier WASM. Root cause was **not** a code bug in
the live system — it was a **contract skew**: the leaf-derivation + merkle fold +
crypto + URN/retrieval-key contract is *mirrored* across layers instead of shared
from one source, so a producer at one version and a verifier at another silently
disagreed (the fold reaches a different root → "not found", no loud error).

### The four skew surfaces (verified)

1. **Symmetric crypto** (AES-256-GCM-SIV chunk en/decrypt, HKDF `derive_decryption_key`,
   `FIXED_NONCE`, HKDF domain constants) lives in `digstore-crypto`, which pulls
   `chia-bls → blst` (does not compile to wasm32). So `dig-client-wasm/crypto.rs`
   **re-implements it byte-for-byte** and `tests/parity.rs` exists only to police drift.
2. **Resource leaf** = `sha256(concat_output(chunks))` in `digstore-store` (producer);
   the verifier hardcodes `sha256(ciphertext)`. Equivalence is implicit/unenforced.
3. **`retrieval_key = sha256(canonical_urn)`** is canonical in `digstore-core::Urn`,
   but reimplemented in `hub/retrieval::seed_from_text` (+ inline URN string) and again
   in the frontend `loader.js` (`sha256Hex(urn)` in pure JS).
4. **No format-version tag** on the proof/module, so a producer/verifier mismatch is
   silent rather than a loud "recompile".

Plus: hub pins `digstore` at `f563e0c` for host/core/crypto, but the **compile binary**
is published out-of-band at `0.5.7` (a different rev) — no enforced single contract
version across producer / host / verifier; and `loader.js`/`dig-client.js` duplicate
chunk-reassembly + base64.

## Decomposition (4 sequenced sub-projects)

- **SP1 — Contract consolidation (this spec).** `digstore-core` becomes the *complete*
  wasm-safe contract: fold the pure read-crypto in + add one shared `resource_leaf()`.
  `digstore-crypto` re-exports core's crypto (keeps BLS + `TamperError`, zero host churn).
  `dig-client-wasm` depends only on `digstore-core`, deletes `crypto.rs`, and the
  producer/verifier call the *same* `resource_leaf`. **Behavior-preserving / byte-identical.**
  Kills surfaces #1 and #2.
- **SP2 — Format-version tag + skew detection.** A `CONTRACT_VERSION` in core, embedded in
  the proof/module envelope; the verifier rejects a version mismatch with a clear
  "generation built with format vN, recompile" error instead of a silent miss. Defines the
  migration for pre-`0.5.7` stores. Kills surface #4. (Wire change → coordinated re-deploy + recompile.)
- **SP3 — Frontend convergence.** One read module; all key-derivation / verify goes through
  the WASM contract (no JS `sha256(urn)`); shared chunk-reassembly + base64 between
  `dig-client.js` and the usercontent `loader.js`. Kills surface #3 (frontend half).
- **SP4 — dighub↔digstore version-lock + CI producer.** Single pinned rev for every
  consumer; build + publish the compile binary AND the verifier WASM in CI from that one
  rev (kills the out-of-band drift that started this); hub calls `Urn::retrieval_key`
  instead of `seed_from_text`. Kills surface #3 (hub half) + the producer drift.

## SP1 design (implementing now)

### Moves
- **`digstore-core/src/crypto.rs` (new, `no_std`):** `derive_decryption_key(canonical_urn,
  Option<&SecretSalt>)`, `encrypt_chunk`, `decrypt_chunk(...) -> Result<Vec<u8>, ()>`,
  with the exact `HKDF_SALT_DOMAIN` / `HKDF_INFO` / `FIXED_NONCE` constants. Deps added to
  core: `aes-gcm-siv` (`default-features=false`, `aes`+`alloc` — no `getrandom`), `hkdf`
  (`default-features=false`). `sha2` already present.
- **`digstore-core/src/merkle.rs`:** `pub fn resource_leaf(ciphertext: &[u8]) -> Bytes32`
  (= `sha256(ciphertext)`) — the single D5 per-resource leaf function used by BOTH the
  producer and the verifier.
- **`digstore-core/src/lib.rs`:** export `crypto::{...}` + `merkle::resource_leaf`.

### Consumers
- **`digstore-crypto`:** `aead.rs`/`kdf.rs` become thin re-exports of `digstore_core::crypto`.
  `decrypt_chunk` keeps its host-facing `TamperError` signature by wrapping core's `Result<_,()>`.
  BLS / fixtures / error unchanged. No host call-site churn.
- **`digstore-store`:** `leaf = digstore_core::resource_leaf(&blob)` (was `digstore_crypto::sha256`).
- **`dig-client-wasm`:** delete `src/crypto.rs`; `Cargo.toml` drops `aes-gcm-siv`/`hkdf`/`sha2`
  direct deps (they come transitively via `digstore-core`, `default-features=false`, so no
  `getrandom`/`blst`). `lib.rs` uses `digstore_core::{resource_leaf, crypto::{derive_decryption_key,
  decrypt_chunk}}`. `tests/parity.rs` simplified to a known-answer + full-pipeline integration test.

### Verification
- `cargo test` across the workspace (core/crypto/store/host) stays green.
- `cargo test` in `dig-client-wasm` (native) — `full_pipeline_single_chunk_round_trip`,
  `decoy_proof_does_not_chain_to_trusted_root` stay green.
- `wasm-pack build crates/dig-client-wasm --target web` compiles (no `getrandom`/`blst`),
  proving the consolidated `digstore-core` is wasm-safe.
- Rebuilt WASM is byte-behavior-identical; SRI updated; redeploy batched with SP2/SP4.

### Why byte-identical matters
The currently-published `0.5.7` binary + deployed verifier already agree (proven by
`dighost_serve` + parity). SP1 only *de-duplicates* — it must not change any committed bytes,
so existing `0.5.7`-compiled stores keep reading and no new skew is introduced.
