# Security

This document records the security posture of the digstore workspace: the
hardening applied in the 2026-06-09 audit, the residual risks that are tracked
but not yet closed, and how to run the supply-chain checks.

## Reporting

Report vulnerabilities privately to the maintainer (see `Cargo.toml` authors).
Do not open public issues for unpatched security bugs.

## Threat model (summary)

- **Storage/serving host is untrusted for confidentiality** ("provider blind"):
  it stores ciphertext and serves it without decrypting. Chunk plaintext and the
  per-URN keys never leave the client.
- **Remote sync peers are untrusted**: a `clone`/`pull` must not be able to
  install or execute attacker-chosen content. Transport is TLS; modules are
  verified against the requested store identity before use.
- **WASM guest modules are untrusted**: the host sandboxes them with memory /
  table / fuel / wall-clock limits and a restricted host-import surface.
- **Store identity is the on-chain Chia launcher id** (§20.1): `store_id` is the
  launcher id of the store's singleton, minted by `digstore init`. It is NOT a
  hash of the publisher key. Head authorization — advancing a store to a served
  root — is a BLS signature over `SHA-256(root || store_id)` verified against the
  module's embedded publisher key.

## Hardening applied (2026-06-09)

Crypto
- **Chunk AEAD switched to AES-256-GCM-SIV (RFC 8452).** The previous
  AES-256-GCM used a fixed all-zero nonce across every chunk and every version of
  a resource; under GCM that is catastrophic (keystream reuse + GHASH
  authentication-key recovery / "forbidden attack"). GCM-SIV is nonce-misuse
  resistant, so a fixed nonce is safe while keeping encryption deterministic for
  the ciphertext-committed merkle root. (`digstore-crypto/src/aead.rs`)
- **Identity (point-at-infinity) BLS public keys are rejected** by
  `validate_public_key`. (`digstore-crypto/src/bls.rs`)

Remote sync (clone / pull / client)
- **Downloaded modules are cryptographically verified before use.** `clone` and
  `pull` now require: embedded `StoreId == requested store id` (the launcher id),
  and the merkle root recomputed from the module's own content equals both the
  embedded `CurrentRoot` and the served root. Previously the client installed and
  executed whatever bytes the server returned. (`digstore-compiler::verify_module_root`,
  wired in `digstore-cli/.../remote_ops.rs`)
- **Authenticated head (closes the former residual #1).** The remote now persists
  the verified publisher push signature per root and returns the served-head
  signature in the store descriptor; `clone`/`pull` re-verify it (`verify_push`)
  against the module's embedded publisher key and **fail closed** on an absent or
  invalid signature. This upgrades clone/pull from "self-consistent module" to
  "publisher-authorized content", so a malicious origin that does not hold the
  publisher's private key can no longer serve fabricated content. A regression test
  (`clone_rejects_unauthenticated_or_forged_head`) proves the fail-closed path.
  (`digstore-remote` wire/backends/handlers, `remote_ops.rs::verify_head_signature`)
- **Transport policy enforced.** Remote URLs must be `https://` (plaintext
  `http://` is allowed only for loopback). The HTTP client follows no redirects,
  so a malicious server cannot bounce a push (signature + body + bearer) to an
  attacker host or use redirects for SSRF. (`remote_ops.rs`, `client.rs`)
- **Delta chunk integrity verified**: each delta chunk must hash to its
  advertised content id before it is accepted. (`client.rs`)
- **Rate limiter is now a time-based token bucket** that refills over a window
  (a one-time burst can no longer permanently lock out a store) and the bucket
  map is bounded (no unbounded-growth memory DoS). (`ratelimit.rs`)
- **5xx responses no longer echo internal detail** (filesystem paths, IO/join
  errors); detail is logged server-side only. (`server.rs`)

Host / WASM sandbox
- **`jwks_fetch` SSRF guard**: the guest-controlled URL must be `https` and
  resolve only to public addresses; loopback/private/link-local/CGNAT and the
  cloud-metadata endpoint are refused. (`digstore-host/src/imports.rs`)
- **Store limits bound all growable resources** (linear memory, table elements,
  table/memory/instance counts), and the WASM threads/shared-memory proposal is
  disabled for serve-only modules. (`runtime.rs`)
- **Fetched modules are size-bounded before validate/compile** in `dighost`.
- **Blind-serve host RNG uses OS entropy** instead of a hardcoded seed, so decoys
  returned on a retrieval miss are not distinguishable from real content.
  (`serve_blind.rs`)

Multi-store workspaces and resource limits
- **Per-store 128 MB content cap.** Each store enforces a hard cap of
  `MAX_STORE_BYTES = 128_000_000` (decimal) on staged plaintext content. It is
  enforced **atomically at `add`** (staging that would exceed the cap stages
  nothing and errors with the remaining headroom) and **defensively at `commit`**.
  The cap bounds the worst-case data-section blob, so a store can never produce a
  module that exceeds the module memory ceiling. (`digstore-core` is the single
  source of truth for the constant; the CLI enforces it.)
- **Module memory ceiling raised to a configurable 384 MiB.** The module-declared
  linear-memory cap is 6144 pages (384 MiB), sized to hold the embedded data
  section (up to the 128 MB cap plus overhead) and a single-copy serve of a
  worst-case ~122 MB resource. The host's outer limit defaults to the same 384 MiB
  and is operator-configurable via `ExecutionLimits.memory_bytes_max` — the real
  DoS bound for an untrusted module. The guest heap base is placed dynamically
  above the data section, so heap growth can never corrupt the embedded chunk pool
  for any blob size. (`digstore-compiler`, `digstore-host`, `digstore-guest`)
- **Uniform module size.** Size-obfuscation filler pads every module's data blob
  to one fixed budget, so every store compiles to the same module size regardless
  of content. A full-cap store carries ~no filler; smaller stores carry
  deterministic filler up to the budget. The module size therefore reveals nothing
  about how much content a store holds. (`digstore-compiler`)

CLI / filesystem
- **Key material uses the OS CSPRNG** (`getrandom`); the previous time/pid/pointer
  "RNG" produced predictable BLS signing keys and private salts. The weak fallback
  was removed (fail closed). (`store_ops.rs`)
- **Secret files (`signing_key.bin`, salt) are written `0600`** on Unix.
- **`checkout` rejects path-traversing resource keys** (`..`, absolute paths,
  Windows drive/ADS), so a malicious cloned store cannot write outside the output
  directory. (`checkout.rs`)

Supply chain / build
- **`Cargo.lock` is now committed** (this workspace ships binaries).
- **`[profile.release]` enables `overflow-checks`** — this code does
  offset/length arithmetic on untrusted serialized input.
- **`deny.toml` added** for `cargo deny check` (advisories, licenses, sources,
  wildcard bans).

## Residual risks / tracked follow-ups

These are known and NOT yet fixed. They are intentionally called out so they are
not mistaken for closed.

1. **Key rotation / root revocation.** With the authenticated head in place
   (below), a leaked store key still cannot be rotated without minting a new store
   id, and there is no signed tombstone to retract a previously published root.
   The store key is effectively long-lived.
2. **Merkle tree has no leaf/node domain separation** (`digstore-core/src/merkle.rs`)
   and **BLS signing messages lack per-role domain-separation tags**
   (`digstore-crypto/src/bls.rs`). Both are defense-in-depth against
   second-preimage / cross-protocol signature reuse. Deferred because the change
   alters every root/signature and must be made in lockstep across the host and
   the guest verifier plus all fixtures.
3. **Proof backend is `MockProver` (forgeable) on the default serve path — but
   the chain source, clock, and backend selection are now real and injectable.**
   - **Real chain source.** `digstore_prover::CoinsetChainSource` fetches the
     current Chia peak + on-chain block records from `https://api.coinset.org`
     (`POST /get_blockchain_state`, `POST /get_block_record_by_height`),
     supplying the real block header hash / height / timestamp the attestation
     freshness gate anchors to (§13.7/§16). Best-effort, short timeout, clear
     `ChainRpc` errors; parsing + HTTP-mocked tests run in CI and an
     `#[ignore]`d live test hits the real mirror.
     (`digstore-prover/src/coinset.rs`, `tests/coinset_parse.rs`,
     `tests/coinset_live.rs`)
   - **Real clock.** `digstore_host::SystemClock` (OS wall clock) replaces the
     `FixedClock` whenever a real chain is wired. (`digstore-host/src/clock.rs`)
   - **Injectable serve path.** `serve_blind` no longer hardcodes the trio.
     `BlindServeDeps` makes the prover, chain source, and clock injectable; it
     **defaults** to the mock/fixed trio (so existing tests + the toolchain-free
     default build stay green) and a caller can swap in
     `CoinsetChainSource` + `SystemClock` (`with_real_chain_clock`) and a real
     `Risc0Prover` (`with_risc0_prover`). `serve_blind_with` is the injection
     entry point. (`digstore-host/src/serve_blind.rs`)
   - **The backend SELECTION compiles in both modes.** With the `risc0` feature
     OFF (the default) the backend is `MockProver`; with it ON, a real
     `Risc0Prover` is available — guarded by `#[cfg(feature = "risc0")]` so the
     default build never pulls the toolchain.

   **Still required for trustworthy proofs (the toolchain boundary):** real
   RISC0 proving needs the **RISC0 toolchain (`r0vm`/`rzup`)** and is enabled via
   the **`risc0` cargo feature** (`digstore-host/risc0` -> `digstore-prover/risc0`),
   which triggers `risc0-build`'s `embed_methods` to compile the zkVM guest ELF.
   It is NOT built or tested in CI here because the toolchain is not installed in
   this environment. The wiring is done: **flip the `risc0` feature, install the
   toolchain, and supply `CoinsetChainSource` + `SystemClock` + `Risc0Prover`**
   to `serve_blind_with` to produce real execution proofs. Until then the default
   serve path's proofs remain forgeable (mock backend).

   **The dig-node read path reports no mock as verified (#126/#134).** On the
   `dig-node` `dig.getContent` live read path the trust-bearing fields are REAL,
   and there is NO execution attestation to fake:
   - **inclusion proof** — REAL: the guest computes the merkle proof from the
     module's own `MerkleNodes` (`build_real_proof`), independent of the prover
     backend (the `MockProver` does NOT touch it);
   - **chain-anchored root** — REAL: the node resolves the CHIP-0035 singleton's
     current on-chain root and serves the matching generation or fails closed
     (`ROOT_NOT_ANCHORED`/-32005, #127), so the root is chain-verified, not mocked;
   - **clock / freshness** — INERT on this path: `dig.getContent` sends
     `window: None` and serves with `require_attestation = false`, so the guest's
     temporal gate (`within_window`) and the host-attestation gate never run — the
     default `FixedClock` therefore cannot affect what is served;
   - **execution proof** — ABSENT: `ContentResponse` carries no execution-proof
     field, and `dig-node` does NOT implement `dig.getProof` (it returns the
     catalogued `-32601` method-not-found rather than fabricating a mock receipt).
   So the dig-node read path never presents a forgeable proof AS verified. A
   verified execution attestation on a live read path remains gated solely on the
   RISC0-toolchain boundary above; the hub/browser shields render that
   mock/absent state honestly (#134). A regression test in `dig-node`
   (`get_content_*` / `get_proof_is_not_served_as_a_verified_proof`) pins this
   honest contract.
4. **JWT signature verification — implemented (closes the former residual #4).**
   The guest JWT gate (`digstore-guest/src/content.rs`) now verifies the token's
   cryptographic signature, not just its claims. RS256 (`rsa` PKCS#1 v1.5 over
   SHA-256) and ES256 (`p256`) are supported; the verifying key is reconstructed
   from the store's trusted JWKS, which the guest fetches over the session-gated
   `jwks_fetch` host import using the `jwks_url` advertised in the embedded
   AuthInfo section (§6.2). The gate is no longer hardcoded off: `get_content`/
   `get_proof` derive `require_jwt` from `AuthInfo.requires_jwt`. A token with a
   valid signature from a trusted key **and** valid claims releases real content;
   a tampered/absent signature, a key not in the JWKS, an unknown `kid`, a missing
   JWKS URL, or any claim failure fails closed -> Decoy (never real content,
   never a 404).
5. **Dependency advisories** currently accepted (see `deny.toml`): `rsa`
   (RUSTSEC-2023-0071, Marvin timing side channel). `rsa` v0.9 is a **direct,
   verify-only** dependency of `digstore-guest` — it is used solely for JWT RS256
   signature verification (public-key `verify`, no decryption or private-key
   operations). The Marvin attack is a timing oracle against RSA *decryption*, so
   it does not apply to signature verification; the ignore is retained with that
   rationale and must be re-evaluated if `rsa` is ever used to decrypt. The former
   `bincode 1.x` advisory (RUSTSEC-2025-0141) ignore has been **removed**: `bincode`
   is no longer in the dependency tree. Re-evaluate each audit.
6. **Clone/pull chain-verified head authentication — Closed (2026-06-11, Phase B).**
   Now that `store_id` is the on-chain Chia launcher id (no longer
   `SHA-256(publisher key)`), the integrity gate (`verify_module_root`) checks
   `StoreId == requested` + merkle self-consistency, and the head signature is
   verified against the publisher key the *module itself carries*. On top of that,
   `clone`/`pull` now verify that the served root **equals the store singleton's
   current on-chain root**, read from the chain via the launcher id embedded in the
   module's `ChainState` section (Phase A). The check **fails closed** on a mismatch
   *or* an unreachable chain (`CliError::VerificationFailed`, exit code 5) — it never
   silently falls back to trusting the served head. The chain (not the serving origin)
   is therefore the authority for the latest authorized root, closing the earlier
   first-use-trust gap: a malicious origin can no longer serve a self-consistent module
   with an attacker-chosen root for the correct launcher id, because the on-chain root
   will not match.

   *Backward-compat caveat:* a module that carries **no embedded `ChainState`** (older
   modules predating Phase A) has no on-chain pointer to verify against, so the
   chain-root gate is a no-op for it and the head-signature gate remains the authority.
   *Offline test seam:* verification is exercised entirely offline via the
   `DIGSTORE_ANCHOR_MOCK` seam (`DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT` /
   `DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE`; see
   `crates/digstore-cli/tests/cli_chain_verify.rs`).

## Running the checks

```sh
cargo test --workspace
cargo install cargo-deny --locked && cargo deny check advisories bans sources
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, build+test on Linux+Windows,
and the `cargo deny` supply-chain checks on every PR and push to `main`. The
`cargo deny` license-compliance check is not yet gated — the workspace crates
need explicit `license` fields and the full transitive license set must be
enumerated first; this is a tracked follow-up.
