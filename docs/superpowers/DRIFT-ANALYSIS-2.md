# Digstore — Drift Analysis (Second Pass) vs `part3_digstore_artifact.pdf`

**Date:** 2026-06-09 · **Tree:** 192 commits, workspace green, guest wasm builds, module self-serves.
**Method:** independent fresh-eyes saturation re-audit (7 read-only auditors reading the actual PDF pages + code + approved-deviation docs), explicitly re-verifying all 14 first-pass fixes are spec-*exact* and hunting regressions. Confirmed drifts fixed TDD; adversarial reverify.

---

## 1. Verdict

**Second pass: zero accidental drift remaining** (after fixes below). 181 audited claims this pass → **147 faithful, ~18 approved deviations, 12 properties**. The first 14 fixes were re-verified **spec-exact with no regressions** (attestation gate confirmed GENUINE — real bls12_381 AugScheme verify + trusted-set membership + freshness, not a stub). The second pass surfaced **5 additional drifts the first pass missed**, all now fixed.

## 2. New drifts found by pass 2 — all FIXED

| id | § | severity | finding | fix (commit) |
|----|---|----------|---------|--------------|
| D-IMPORT-SECTION | 5.1 | medium | baked template fixture had no `dig_host` Import section → default-emitted module lacked §5.1 imports | emit §5.1 Import section with all 8 imports (`098e6bf`) |
| D-COMPILER-VERSION | 5 | low | paper states compiler v1.0.0; nothing carried/embedded it | carry compiler version 1.0.0 in the artifact (`8605774`) |
| D-MEMORY-MIN-PAGES | 5.1 | low | template min was 4 pages, not the §5.1 nominal 1 | template memory min = 1 page (`a33d68d`) |
| D-DECOY-OCTAVE-BITS | 14.2 | low | decoy octave used bits 40-63, leaving bits 32-39 dead; doc inaccurate | octave uses top 3 bits, no dead seed bits (`8632721`) |
| **§17.1 obfuscation** | 17.1 | medium | **no-op marker** — copied all sections verbatim + appended a string naming the 4 techniques, performing none | **real deterministic behavior-preserving passes** (`ba8ea1a`) |

### Obfuscation (§17.1) — now genuine
`crates/digstore-compiler/src/obfuscate.rs` now performs real WASM transformation:
- **control-flow nops** — deterministic `nop` insertion between operators in each function body.
- **opaque predicates** — every body prefixed with an always-true, stack-neutral, empty-blocktype guard (shifts no branch depths).
- **bogus code** — 8 unreferenced dead functions appended (new type + function + code entries), never called/exported.
- **instruction substitution** — *documented-deferred*: a general semantics-preserving operator substitution could not be proven sound for every reachable context within scope; per "omit rather than risk behavior," it is omitted and noted in the source doc comment. The other three are genuine, so the pass is no longer a no-op.

**Gates (all pass):** obf-on vs obf-off compile → instantiate both via `HostRuntime` → **byte-identical `serve_content`** for hit and miss; obfuscated module still self-serves with a verifying proof; double-compile byte-identical (determinism); valid wasm; structural-transformation guard tests (nop count + function count strictly increase) so a future no-op regression is caught.

## 3. First-pass fixes re-verified spec-exact (no regression)
§12 attestation (verify/trustset/nonce/freshness), §6.3/§12.4 session-expired + JWT-gate-requires-session, §5.1 memory max/memory64/shared enforcement, §8.5 default-resource + well-known manifest, §9.5 proof-path-len (documented bound), §13.7 node-key bound to attestation key, §18.4 host/client boundary. All confirmed correct in code with `file:line` evidence.

## 4. Approved deviations re-confirmed (unchanged, documented)
Big-endian Chia codec (D1), deterministic ChaCha20 filler (D2/§19.3), risc0 re-execution + `program_hash=SHA256(module)` (D3), per-resource merkle leaf (D5), filler section (D4), fixed GCM nonce under per-URN keys (#4), proof size ≤ ceil(log2 n) (#5/D8), data-section offset (D2), wasmtime 45, ProofPrelude/mock-prover-default (C3). Code matches each.

## 5. Residual note
The only intentionally-incomplete item is obfuscation **instruction substitution** (1 of 4 techniques), deferred as not provably behavior-preserving in scope — explicitly documented in code. The paper marks obfuscation optional and states security does not rest on it (§17.1); §17.2 secretlessness (the load-bearing property) is genuinely verified. A provably-safe constant-decomposition substitution could close this in a follow-up if strict 4/4 is required.

## 6. Three deliverable artifacts (all runnable)
1. `digstore.exe` — git-like binary. 2. `{storeID}-{root}.wasm` — self-serving datastore (`commit` output). 3. `dighost.exe` — S3-compatible host (object_store: AWS S3 / MinIO / local), streams content by retrieval key, never decrypts.
