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
- **Store identity is self-certifying**: `store_id == SHA-256(store BLS public
  key)` (§20.1). Authorization to advance a served root is a BLS signature over
  `SHA-256(root || store_id)`.

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
  `pull` now require: embedded `StoreId == requested store id`, `SHA-256(embedded
  PublicKey) == StoreId`, and the merkle root recomputed from the module's own
  content equals both the embedded `CurrentRoot` and the served root. Previously
  the client installed and executed whatever bytes the server returned.
  (`digstore-compiler::verify_module_root`, wired in `digstore-cli/.../remote_ops.rs`)
- **Authenticated head (closes the former residual #1).** The remote now persists
  the verified publisher push signature per root and returns the served-head
  signature in the store descriptor; `clone`/`pull` re-verify it (`verify_push`)
  against the store-id-bound module key and **fail closed** on an absent or
  invalid signature. This upgrades clone/pull from "self-consistent module" to
  "publisher-authorized content", so a malicious origin holding only the public
  store key can no longer serve fabricated content. A regression test
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
3. **Proof backend is `MockProver` (forgeable) on the default serve path**, with
   a `FixedClock` and `MockChainSource`. The RISC0 backend must be wired and a
   real chain/clock supplied before execution proofs are trustworthy.
   (`digstore-host/src/serve_blind.rs`)
4. **JWT gate validates claims but does not verify token signatures**
   (`digstore-guest/src/content.rs`); the gate is also hardcoded off. Do not rely
   on JWT authorization until signature verification is implemented.
5. **Dependency advisories** currently accepted (see `deny.toml`): `rsa`
   (RUSTSEC-2023-0071, Marvin timing side channel, transitive) and `bincode 1.x`
   (RUSTSEC-2025-0141, unmaintained, RISC0 proof path). Re-evaluate each audit.

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
