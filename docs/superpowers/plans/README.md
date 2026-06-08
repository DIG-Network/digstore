# Digstore — Implementation Plan (Master Index)

Full-fidelity Rust implementation of *Digstore: The Content-Addressable WASM Store Format* (`part3_digstore_artifact.pdf`).

- **Design spec:** [`../specs/2026-06-08-digstore-design.md`](../specs/2026-06-08-digstore-design.md) — locked tech decisions + documented deviations.
- **Binding conventions:** [`00-CONVENTIONS.md`](00-CONVENTIONS.md) — **overrides** per-crate plans on cross-crate interfaces. Read first.
- **Properties / doc-only sections:** [`00-PROPERTIES.md`](00-PROPERTIES.md) — §8.5, §9.4, §10, §15, §17.2.

## Execution method
subagent-driven-development, TDD, frequent conventional commits. Build bottom-up in the dependency order below. Each plan is self-contained bite-sized TDD steps.

## Dependency order (acyclic)
```
1. digstore-core      (no_std + alloc; types, codec, URN, ABI, merkle)
2. digstore-chunker   ← core
3. digstore-crypto    ← core            (BLS module = C1 blocker fix)
4. digstore-store     ← core, chunker, crypto
5. digstore-guest     ← core, crypto    (wasm32; serves; ProofPrelude per C3)
6. digstore-prover    ← core, crypto    (Prover/Verifier + ChainSource; mock default)
7. digstore-host      ← core, crypto, prover
8. digstore-compiler  ← core, store, guest(prebuilt wasm)
9. digstore-remote    ← core, store, host, crypto
10. digstore-cli      ← all
```
(core → {chunker,crypto} → store → {guest,prover} → host → {compiler,remote} → cli)

## Plans
| # | Crate | Plan | Paper sections |
|---|-------|------|----------------|
| 1 | digstore-core | [`01-digstore-core.md`](01-digstore-core.md) | 5.2(structs), 6.1, 6.5, 7.1–7.3, 8.4, 9.1–9.3, 9.5 |
| 2 | digstore-chunker | [`02-digstore-chunker.md`](02-digstore-chunker.md) | 8.1, 8.2(chunking) |
| 3 | digstore-crypto | [`03-digstore-crypto.md`](03-digstore-crypto.md) | 11.1–11.4, 12(sign), 13.7, 21.6 |
| 4 | digstore-store | [`04-digstore-store.md`](04-digstore-store.md) | 4.1–4.4, 8.2, 20.1–20.4(mechanics) |
| 5 | digstore-guest | [`05-digstore-guest.md`](05-digstore-guest.md) | 5.1, 6.1–6.3, 8.3, 12.1–12.4, 14.1–14.4, 16 |
| 6 | digstore-prover | [`08-digstore-prover.md`](08-digstore-prover.md) | 13.1–13.8 |
| 7 | digstore-host | [`07-digstore-host.md`](07-digstore-host.md) | 6.3, 6.4, 12(host), 13.6, 18.1–18.4 |
| 8 | digstore-compiler | [`06-digstore-compiler.md`](06-digstore-compiler.md) | 5.1–5.3, 8.3, 17.1, 19.1–19.4 |
| 9 | digstore-remote | [`09-digstore-remote.md`](09-digstore-remote.md) | 21.1–21.8 |
| 10 | digstore-cli | [`10-digstore-cli.md`](10-digstore-cli.md) | 20.1–20.7, 11.3, 9.3, 14.2(client), 11.4 |

(File numbering 06=compiler, 08=prover is historical; build order above is authoritative.)

## Coverage
Every implementable section 4.1–21.8 has an owner — see [`_coverage.json`](_coverage.json) for the full matrix. Doc-only sections (8.5, 9.4, 10, 15, 17.2) are realized + verified per [`00-PROPERTIES.md`](00-PROPERTIES.md).

## Cross-crate interface fixes (resolved in CONVENTIONS)
- **C1 (blocker):** `digstore-crypto::bls` module exports `SecretKey`/`PublicKey`/`Signature` (+ alias `BlsSecretKey`); host & prover consume these.
- **C2:** core publishes submodule paths + flat re-exports + `types` alias.
- **C3:** guest returns `ProofPrelude`; host+prover build the `ExecutionProof`.
- **C4–C10:** §16 owner, 20.4 partition, stats naming, single push-signing message, BLS parity fixtures, guest↔prover output parity, single KDF.

## Cross-cutting verification
Determinism (double-compile byte-identical), BLS cross-impl parity fixtures, ABI golden tests, decoy real-vs-miss shape equality, codec Chia-streamable round-trips, end-to-end init→add→commit→cat→clone→push→pull.
