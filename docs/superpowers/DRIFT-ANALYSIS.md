# Digstore — Drift Analysis vs `part3_digstore_artifact.pdf`

**Date:** 2026-06-08 · **Tree:** 170 commits, workspace green, guest wasm builds, module self-serves (adversarially verified).
**Method:** every implementable paper section mapped to code evidence (greps + reads against the committed tree). Classification:

- **Faithful** — code matches the paper.
- **Deviation (documented)** — intentional, recorded in design spec / CONVENTIONS / DATASECTION-CONTRACT.
- **Drift (accidental/partial)** — diverges with no/weak justification → action recommended.
- **Property** — §10/§15 etc.: a guarantee verified by tests, not a standalone unit.
- **Missing** — paper claim with no implementation.

---

## 1. Verdict

The implementation is **faithful to the whitepaper across all of Parts 2–4** with **8 intentional, documented deviations** and **2 accidental/partial drifts** (one low-medium, one low). No security-critical behavior is missing: confidentiality (URN-derived keys, client-only decrypt), integrity (verifying merkle proof to trusted root + GCM tags + execution proof), provider blindness (hash-addressed ciphertext), indistinguishability (decoys + oblivious access), attestation, sessions, secretlessness, and the self-serving module are all genuinely implemented and tested.

**Action items (2):** (A) §17.1 obfuscation is a no-op placeholder; (B) §8.5 social-convention behavior (default `index.html`, discovery manifest) is absent. Both are explicitly low-stakes in the paper (obfuscation "security does not rest on it"; conventions are opt-in), but both diverge from implementation intent and are listed in §4 below.

---

## 2. Section-by-section matrix

| § | Claim | Status | Evidence |
|---|-------|--------|----------|
| 4.1 | `StoreConfig`, `Visibility::{Public, Private(SecretSalt)}` | Faithful | `core/src/config.rs:16` enum Visibility Public/Private |
| 4.2 | 32-byte store ID, hex | Faithful | `Bytes32`; store_id_hex in `store/src/...` |
| 4.3 | Generations + monotonic root history | Faithful | `GenerationState{id,root,timestamp}`, `roots.log` append-only |
| 4.4 | On-disk layout `~/.dig/...` | Faithful | `store` `StorePaths` builds exact tree (layout test) |
| 5.1 | Module sections; mem min 1 / max 256 pages (16 MiB) | Faithful | `config.rs:62` max 256; memory section in template |
| 5.2 | Embedded: chunks, key table, metadata, manifest, trusted keys, **no secret** | Faithful | data-section ids 1–11; secretless test |
| 5.3 | 10-stage compilation pipeline | Faithful | `compiler/src/pipeline.rs` |
| 6.1 | `pack_ptr_len`/`unpack_ptr_len`/`is_error` | Faithful (exact) | `core/src/abi.rs`, golden tests |
| 6.2 | 12 exports | Faithful | all present: get_store_id…get_proof, alloc, dealloc, init, memory |
| 6.3 | 8 `dig_host` imports; `jwks_fetch` session-gated | Faithful | `guest/src/imports.rs`; host returns NoSession before session |
| 6.4 | Return buffer 64 KiB default / 16 MiB max | Faithful (exact) | `config.rs:81-82` |
| 6.5 | ErrorCode values | Faithful (exact) | `error.rs`: -1,-2,-3,-100,-101,-102,-200,-203,-300,-301 |
| 7.1–7.3 | URN format; canonical; `retrieval_key = SHA-256(canonical)` | Faithful | `core/src/urn.rs` |
| 8.1 | CDC gear; 16/64/256 KiB | Faithful (exact) | `chunker`; `config.rs:60-62` |
| 8.2 | Generations; cross-gen dedup | Faithful | store: shared-chunk-stored-once test |
| 8.3 | Interleaved pool, no resource boundaries, filler | **Deviation (documented)** | pool = global-index sequential (no resource grouping); filler in separate section id 11, not interleaved in gaps — DATASECTION-CONTRACT D4 |
| 8.4 | Metadata manifest, plaintext, ungated | Faithful | `MetadataManifest` all fields; `get_metadata` ungated |
| 8.5 | Social conventions (`index.html`, `/.well-known/dig/manifest.json`) | **Drift (partial/missing)** | only test resource keys; no default-resource fallback in `cat`, no discovery-manifest helper |
| 9.1–9.3,9.5 | Merkle leaf=SHA256(chunk), node=SHA256(l‖r), odd carry, verify | **Deviation (documented)** | leaf = SHA-256(**resource** ciphertext), not per-chunk — DATASECTION-CONTRACT D5; node/odd-carry/verify faithful |
| 9.4 | generation root = tree root | Faithful | store: `state.root == tree.root()`; PROPERTIES |
| 10 | Threat model | Property | mechanisms mapped in PROPERTIES |
| 11.1 | HKDF-SHA256 from URN | Faithful | `crypto/src/kdf.rs` |
| 11.2 | AES-256-GCM, fixed nonce | Faithful | `aead.rs` FIXED `[0u8;12]`; unique key per URN |
| 11.3 | Client-side decryption | Faithful | cli client_crypto; guest never decrypts |
| 11.4 | Private store salt | Faithful | `Private(SecretSalt)`; cli without-salt fails GCM |
| 12.1–12.4 | Attestation challenge/response, BLS G1 48 / G2 96, trusted key `dig-host-key-v1`, sessions | Faithful | `wire.rs` 48/96; guest verifies via bls12_381; host signs via blst |
| 13.1–13.8 | Execution proof structure, prove/verify, nonce, freshness, node sig, chain anchor, TEE alt | Faithful + **Deviation #3 (documented)** | prover: MockProver default + real risc0 (feature); CoinsetChainSource; HardwareAttest; `program_hash=SHA-256(module)` re-execution model |
| 14.1–14.4 | Decoys (log size, deterministic), oblivious (pad/cover/shuffle), re-randomized | Faithful | `guest/src/decoy.rs`, `oblivious.rs` (pad/cover/shuffle/real_positions) |
| 15 | Provider blindness | Property | host serves hash-addressed ciphertext; verified by self-serve e2e |
| 16 | Temporal validity window | Faithful | `guest/src/temporal.rs` `within_window` → decoy if outside |
| 17.1 | Obfuscation: substitution, opaque predicates, bogus code, control-flow nops | **Drift (no-op placeholder)** | `obfuscate.rs`: verbatim section passthrough + marker custom section; performs none of the 4 |
| 17.2 | Secretless module | Faithful (Property) | `compiler/tests/secretless.rs` |
| 18.1–18.4 | wasmtime runtime, bounds, return buffer, serve flow | Faithful + **Deviation (documented)** | wasmtime 45 (was 27, Windows trap-unwind fix); epoch/fuel/StoreLimits; serve flow 18.4 |
| 19.1–19.4 | Inputs, trusted keys (≥1 or NoTrustedKeys), determinism, atomic write | Faithful | double-compile byte-identical test; NoTrustedKeys; temp+rename |
| 20.1–20.7 | CLI verbs init/add/commit/status/log/diff/checkout/cat/remote/clone/push/pull | Faithful (+extras) | `cli.rs` Command enum: all 12 + List/Remove |
| 21.1–21.8 | REST surface, ETag/304, status codes, push auth (FF-only, pending head), delta | Faithful | server routes /module,/roots,/content,/proof,/delta; 201/202/304/401/403/404/409/413/422/429; push delegates to crypto (C7) |

---

## 3. Documented deviations (intentional — already recorded)

| # | Deviation | Rationale | Recorded in |
|---|-----------|-----------|-------------|
| 1 | Data-section codec **big-endian** (Chia streamable), not paper's "little-endian" | User-required Chia compatibility | design spec §3.1, DATASECTION-CONTRACT |
| 2 | Interleaved-pool filler is a **deterministic ChaCha20** stream, not true random | §19.3 byte-identical recompile requires determinism | design spec §3.2 |
| 3 | Execution proof = risc0 **re-execution** of the serving computation; `program_hash = SHA-256(module)` | risc0 proves RISC-V, not WASM opcodes; mock prover default | design spec §3.3 |
| 4 | Merkle leaf = SHA-256(**resource** ciphertext), not per-chunk | `ContentResponse` carries one proof for the whole served resource | DATASECTION-CONTRACT D5 |
| 5 | Filler lives in a **separate section** (id 11), not interleaved into pool gaps | Interleaving would break global chunk indexing; no-resource-boundary property still holds | DATASECTION-CONTRACT D4 |
| 6 | Data section injected at fixed offset `DIGS_DATA_OFFSET` (0x200000), self-describing | Implementation mechanism for a self-serving module; relocated above guest heap | DATASECTION-CONTRACT D2 |
| 7 | wasmtime 45 (plan pinned 27) | wasmtime 27 fuel/epoch libcall traps crash (non-unwinding) on Windows+rustc 1.94 | host deviations |
| 8 | GCM fixed nonce `[0u8;12]` | Paper-specified; safe because key is unique per URN | §11.2 (faithful) |

---

## 4. Accidental / partial drift (action items)

### A. §17.1 Obfuscation is a no-op placeholder — *severity: low-medium*
`crates/digstore-compiler/src/obfuscate.rs` copies every section **verbatim** (the code section is byte-identical passthrough) and appends one deterministic custom section whose bytes are the literal string `opaque-predicates;bogus-code;control-flow-nops;instruction-substitution`. **None of the four transforms are performed.** The `obfuscation_preserves_exports` / `_is_deterministic` / `_changes_the_bytes` tests pass trivially (the only change is the appended marker).

- **Impact:** none on security — the paper itself states (§17.1) "the format's security does not rest on it," and the load-bearing property §17.2 (secretless) is genuinely verified. But it does not deliver the reverse-engineering cost the plan intended.
- **Fix options:** (1) implement at least one real behavior-preserving pass — e.g. opaque-predicate injection at the existing guest `obfuscation_hooks::opaque_true()` seam, or function-local instruction substitution / control-flow nop insertion in the code section via `wasm-encoder`, keeping determinism + the obf-vs-non-obf identical-`get_content`-output test; or (2) honestly redesignate it in docs as "obfuscation hook (no-op v1), real passes deferred." Recommended: (1), starting with opaque predicates + nop padding (cheapest behavior-preserving pair).

### B. §8.5 Social conventions not implemented as behavior — *severity: low*
`index.html` and `/.well-known/dig/manifest.json` appear only as test resource keys. There is **no** default-resource fallback (`cat <urn-without-resourceKey>` → `index.html`) and **no** discovery-manifest writer/reader. Conventions still *function* (a known resource key can be `cat`'d), but the PROPERTIES doc claimed CLI ownership of the default + discovery behavior, which is absent.

- **Impact:** low — opt-in convenience layer; privacy guarantees unaffected.
- **Fix:** in `digstore-cli`: when `cat` is given a URN with no `resource_key`, resolve `index.html` if present; add `digstore add --discovery` (or commit-time) helper that writes `/.well-known/dig/manifest.json` listing publisher-elected resources. Add the two PROPERTIES tests named in `00-PROPERTIES.md`.

---

## 5. Explicitly out of scope (paper §23) — correctly absent
Network distribution across many providers, external identity anchoring of the store ID, payment settlement. None implemented; none expected.

---

## 6. Recommendation
Ship the current tree as a faithful local-format + HTTPS-remote implementation. Schedule the two action items (A obfuscation passes, B social conventions) as a small follow-up; neither blocks correctness or security. All eight deviations are sound and documented.
