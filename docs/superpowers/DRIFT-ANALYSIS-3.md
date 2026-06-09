# Digstore — Drift Analysis (Third Pass) vs `part3_digstore_artifact.pdf`

**Date:** 2026-06-09 · **Tree:** 195 commits, workspace green (118 test binaries, 0 failures), guest wasm builds, module self-serves, all 3 artifacts build.
**Method:** third independent saturation re-audit (7 read-only auditors reading the actual PDF pages + code + approved-deviation docs), re-verifying every prior fix spec-exact, hunting new drift; confirmed drifts fixed TDD; adversarial reverify.

---

## 1. Verdict — ZERO ACCIDENTAL DRIFT

After the pass-3 fixes below, **no accidental drift remains**. 253 claims audited this pass (~152 consolidated rows) → **132 faithful, 12 approved deviations, 7 properties**, `prior_fixes_ok = true`. Both security gates re-confirmed GENUINE: attestation (real BLS12-381 pairing verify + trusted-set membership + freshness + real challenge nonce) and the now-real obfuscation.

## 2. Pass-3 findings — all FIXED

| id | § | severity | finding | fix (commit) |
|----|---|----------|---------|--------------|
| D-AUTH-01 | 5.2 / 4.1 | medium | `Compiler::compile` took no auth parameter and unconditionally embedded no-auth `AuthenticationInfo` → per-store JWT/session policy could never be compiled into a module | thread per-store `AuthenticationInfo` into `Compiler::compile` (`b649e38`) |
| §17.1 instruction-substitution | 17.1 | low | obfuscation marker advertised "instruction-substitution" while that 4th technique was deferred (code/claim mismatch) | real, provably-equal substitution: `i32.const k` → `i32.const a; i32.const b; i32.add`, `a⊕b≡k` wrapping; deterministic, behavior-preserving (`4e39fb5`) |

### Obfuscation §17.1 — now 4/4 genuine
`crates/digstore-compiler/src/obfuscate.rs` implements all four named techniques for real, deterministically, behavior-preservingly:
1. **control-flow nops** — deterministic `nop` insertion by index parity.
2. **opaque predicates** — always-true, stack-neutral, empty-blocktype guard prefix.
3. **bogus code** — 8 unreferenced dead functions (coherent type/function/code rebuild).
4. **instruction substitution** — every 3rd `i32.const k` → `i32.const a; i32.const b; i32.add` with `a.wrapping_add(b)==k` (exact WASM i32.add semantics; net value + stack effect identical). `a` index-derived, no RNG.

**Gates (pass):** obf-on vs obf-off → byte-identical `serve_content` (hit + miss); double-compile byte-identical (determinism); valid wasm; obfuscated module still self-serves with verifying proof; structural-transform guards (nop/bogus-fn/i32.add counts strictly increase). New test `obfuscation_performs_real_instruction_substitution` (RED→GREEN).

## 3. All prior fixes (passes 1 & 2) re-verified spec-exact, no regression
§12 attestation (BLS verify + trusted-set + nonce + freshness), §6.3/§12.4 SessionExpired vs NoSession + JWT-gate-requires-session, §5.1 Import section (8 dig_host imports) + memory max==256/min==1/memory64==false/shared==false enforced on emitted module, §5 compiler version 1.0.0, §8.5 default-resource + well-known manifest, §9.5 proof-path-len bound, §13.7 node-key bound to attestation key, §18.4 host/client boundary, §14.2 decoy octave bits. All confirmed in code with `file:line` evidence.

## 4. Approved deviations (12) — unchanged, code matches each
Big-endian Chia codec (D1), deterministic ChaCha20 filler (D2/§19.3), risc0 re-execution + `program_hash=SHA256(module)` (D3), per-resource merkle leaf (D5), filler section (D4), fixed GCM nonce under per-URN keys (#4), proof size ≤ ceil(log2 n) (#5/D8), data-section offset 2 MiB (D2), wasmtime 45 (Windows trap fix), ProofPrelude/mock-prover-default (C3), wasm-opt skipped for determinism (§5.3 S8), configurable `data_dir` vs illustrative `~/.dig` (§4.4).

## 5. Three deliverable artifacts — all runnable
1. **`digstore.exe`** (20.8 MB) — git-like binary: init/add/commit/status/log/diff/checkout/cat/remote/clone/push/pull.
2. **`{storeID}-{rootHash}.wasm`** (~600 KB) — the self-serving datastore produced by `digstore commit`.
3. **`dighost.exe`** (18.3 MB) — S3-compatible host (`object_store`: AWS S3 / MinIO / local), streams content by retrieval key, never decrypts; S3 Object Lambda documented as the in-S3 deployment wrapper.

## 6. Conclusion
Three independent drift passes (357 → 181 → 253 claims) drove the implementation to **zero accidental drift** vs the whitepaper, with 12 sound, documented deviations. Security-critical behavior — confidentiality, integrity (verifying merkle proof to trusted root + GCM + execution proof), provider blindness, indistinguishability, genuine host attestation, secretlessness, and the self-serving module — is implemented and tested. Ship-ready.
