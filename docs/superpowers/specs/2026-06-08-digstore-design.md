# Digstore — Rust Implementation Design Spec

**Source:** `part3_digstore_artifact.pdf` — *Digstore: The Content-Addressable WASM Store Format*, Michael Taylor, v1.0, May 2026.
**Date:** 2026-06-08
**Scope decision:** Full-fidelity. Every section of the paper is implemented, including ZK execution proofs, code obfuscation, and oblivious access.
**Status:** Approved design → feeds the phased implementation plan.

---

## 1. Summary

Digstore is a content-addressable, encrypted-at-rest store that compiles into a single self-serving WebAssembly module. The module is **both the data and the server**: its data section holds chunked, encrypted content + merkle commitments + root history + store public key + trusted-host keys; its code section serves content through a fixed export ABI. The module embeds **no secret** — security lives in URNs, not the artifact.

Two values derive from a URN and nothing else:
- `retrieval_key = SHA-256(canonical_urn)` — locates a resource (only thing that leaves the client).
- `decryption_key = HKDF-SHA256(canonical_urn[, salt])` — AES-256-GCM key, used **client-side only**.

A provider serving the module holds opaque ciphertext keyed by hashes; it never sees the URN → **provider blindness is structural, not policy**.

---

## 2. Locked technology decisions

| Concern | Decision |
|---------|----------|
| Build target | Full-fidelity, all paper sections |
| ZK backend | **RISC Zero (risc0)**, behind a `Prover`/`Verifier` trait + mock prover default |
| WASM build strategy | **Template guest + data injection**: serving logic written once as a Rust `guest` crate → wasm32; compiler injects the data section |
| Data-section codec | **Custom codec using Chia streamable framing** (big-endian length prefixes, `Optional` tag byte, `List` 4-byte BE count, raw 32-byte hashes) |
| Oblivious access | **Padded + shuffled scan with cover accesses**, re-randomized per execution via `host_random_bytes` |
| BLS | Guest verifies with pure-Rust **bls12_381**; host signs with **chia-bls (blst)**; same Chia scheme (G1 48-byte pubkeys, G2 96-byte sigs) |
| Remote | **axum** (tokio) server implementing full §21 REST + **reqwest** client |
| JWT auth | **Full RS256 + ES256 validation inside the guest** (pure-Rust rsa/p256), session-gated `jwks_fetch` |
| Chia anchoring | **`ChainSource` trait** → **coinset.org** HTTP impl + deterministic mock |

---

## 3. Resolved contradictions / deviations from the paper

These are deliberate, documented departures forced by physics, determinism, or tooling. Each must survive the user-review gate.

1. **Endianness.** Paper §5.3 says "binary codec, little-endian." Chia's serialization protocol is **big-endian**. The user requires Chia compatibility. **Resolution:** adopt Chia streamable framing throughout the data-section codec — 4-byte big-endian length prefixes, `Optional` = 1 tag byte then value, `List`/`Vec` = 4-byte BE count then items, fixed-width integers big-endian, `Bytes32`/`Bytes48`/`Bytes96` as raw bytes. *Deviation from paper's "little-endian" note, in favor of the explicit Chia-compat requirement.*

2. **"Random filler" vs deterministic compilation.** §8.3 says gaps in the interleaved pool are "random filler"; §19.3 requires byte-identical output from identical inputs. Pure randomness breaks determinism. **Resolution:** filler is a **deterministic CSPRNG stream** (ChaCha20 keyed by `SHA-256(store_id ‖ roothash ‖ "digstore-filler-v1")`). Indistinguishable from random to anyone without the seed derivation, but reproducible. *Deviation: "random" interpreted as "deterministically pseudo-random."*

3. **risc0 proves RISC-V, not WASM.** §13 wants a ZK proof that "running the module whose hash is `program_hash` produces the output." risc0 proves RISC-V execution. **Resolution:** `program_hash = SHA-256(module_bytes)`. The risc0 guest re-executes the **deterministic serving computation** (resolve retrieval key → key-table lookup → gather + concatenate chunk ciphertext → commit output) over inputs that include `program_hash`, the relevant embedded data, and `public_input` (client nonce + coinset.org block ref). The proof attests this computation; `public_output` commits to the returned bytes. The **mock prover is the default** so the rest of the system is fully functional while the real risc0 circuit matures. *Deviation: proof is over a faithful re-execution of the serving computation, not over wasmtime opcodes.*

4. **GCM fixed nonce.** §11.2 mandates a fixed nonce, justified because the key is unique per URN. Implemented exactly as written; documented as safe **only** under the unique-key-per-URN invariant, which the codebase enforces (one key per canonical URN, never reused across plaintexts).

---

## 4. Crate architecture (single Cargo workspace)

```
digstore_wasm/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ digstore-core/          # shared, no_std-compatible
│  ├─ digstore-chunker/       # content-defined chunking
│  ├─ digstore-crypto/        # host-side crypto (sha2/hkdf/aes-gcm/bls)
│  ├─ digstore-guest/         # the served WASM logic → wasm32-unknown-unknown
│  ├─ digstore-compiler/      # dig-compiler: data injection, obfuscation
│  ├─ digstore-store/         # dig-store + config: entity, generations, on-disk
│  ├─ digstore-host/          # dig-host: wasmtime runtime + imports
│  ├─ digstore-prover/        # Prover/Verifier + ChainSource traits & impls
│  ├─ digstore-remote/        # axum server + reqwest client
│  └─ digstore-cli/           # `digstore` binary
└─ docs/
```

### 4.1 `digstore-core` (no_std + alloc)
Shared by guest, host, compiler, CLI. Feature-gated so the guest pulls only what it needs.
- `Bytes32`, `Bytes48`, `Bytes96` newtypes; hex encode/decode.
- **URN**: `Urn { chain, store_id, root_hash: Option, resource_key: Option }`; parse from `urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]`; **canonicalize**; `retrieval_key()` = SHA-256(canonical).
- **ABI**: `pack_ptr_len`, `unpack_ptr_len`, `is_error`; `ErrorCode` enum (exact values from §6.5).
- **Data-section codec** (Chia streamable framing): `Encode`/`Decode` traits + primitives; section table layout (magic `b"DIGS"`, format version `1`, offset table).
- **Merkle**: `MerkleProof`, `ProofStep`, tree build, inclusion verify (§9: leaf=SHA-256(chunk), node=SHA-256(left‖right), odd carried up).
- **Manifest**: `MetadataManifest` + `Author` (§8.4 schema).
- **Key table**: `KeyTableEntry { static_key, generation, chunk_indices, total_size }`.
- Request/response wire structs: `ContentResponse`, `ProofResponse`, `ExecutionProof`, `ChiaBlockRef`, `AttestationChallenge`, `AttestationResponse`, `AuthenticationInfo`.

### 4.2 `digstore-chunker` (host)
- Gear-based rolling hash (FastCDC line). `ChunkerConfig { min_size:16KiB, target_size:64KiB, max_size:256KiB, mask }`.
- Streaming chunk boundaries; each chunk hashed SHA-256 (its content address). Deterministic boundaries for dedup.

### 4.3 `digstore-crypto` (host)
- SHA-256 (sha2), HKDF-SHA256 (hkdf), AES-256-GCM (aes-gcm) — host/CLI side encryption + decryption helpers.
- BLS keygen/sign via chia-bls(blst); Chia AugScheme. Used by host attestation signing, node proof signing, push-auth signing.

### 4.4 `digstore-guest` (wasm32-unknown-unknown, no_std + custom alloc)
**The served logic.** Compiled once → template module the compiler injects data into.
- Exports (§6.2): `get_store_id`, `get_current_roothash`, `get_roothash_history`, `get_public_key`, `get_metadata`, `get_authentication_info`, `get_content`, `get_proof`, `alloc`, `dealloc`, `init`, `memory`.
- Imports (§6.3, `dig_host`): `host_get_public_key`, `host_create_attestation`, `host_establish_session`, `host_verify_session`, `jwks_fetch`, `host_get_current_time`, `host_random_bytes`, `host_read_return_buffer`.
- Linear memory: min 1 page, max 256 pages (16 MiB) (§5.1).
- **Attestation** (§12): issue challenge, verify host BLS sig under trusted set (bls12_381), freshness check; refuse → decoys.
- **Sessions** (§12.4): establish after attestation; gate `jwks_fetch`.
- **JWT auth** (§6.3): RS256 + ES256 verify (rsa/p256), JWKS parse, exp/nbf/aud/iss; decides real vs decoy.
- **Content path** (§7,8,14): canonical retrieval-key lookup in key table → **oblivious** gather from interleaved pool (padded+shuffled+cover reads, re-randomized per call) → return ciphertext + merkle proof.
- **Decoys** (§14.2): miss → deterministic bytes, size from logarithmic distribution seeded by retrieval key, real-looking proof blob, success status. Same miss → same bytes.
- **Temporal keys** (§16): check request window against `host_get_current_time`; outside window → decoy.

### 4.5 `digstore-compiler` (host) — `dig-compiler`
Pipeline (§5.3): config load → generation load (build per-gen merkle) → chunk dedup (global index) → key-table build → **load prebuilt guest template** → inject data section (chunks pool with deterministic filler, key table, metadata, manifest, trusted keys) via wasm-encoder/wasmparser → obfuscation passes (§17, WASM-level: instruction substitution, opaque predicates, bogus code, control-flow nops) → optional wasm-opt → re-validate → **deterministic atomic write** `{hex(store_id)}-{hex(roothash)}.wasm`.
- Refuses empty trusted-key set (`CompilerError::NoTrustedKeys`).
- **Determinism harness**: compile twice → byte-identical (§19.3).

### 4.6 `digstore-store` (host) — `dig-store` + `dig-store-config`
- `StoreConfig { store_id, data_dir, max_size, visibility }`; `Visibility::{Public, Private(SecretSalt)}`.
- `GenerationState { id, root, timestamp }`, `Generation { state, tree }`, monotonic root history.
- On-disk layout (§4.4): `~/.dig/{store_id}.staging.bin`, `generations/{roothash}/manifest.json` + `chunks/{hash}`, `modules/{store_id}-{roothash}.wasm`, `config.toml`.
- Staging area (binary), commit → new generation + compile.

### 4.7 `digstore-host` (host) — `dig-host`
- wasmtime engine (sync), instantiate + validate module, wire `dig_host` imports.
- Shared return buffer (§6.4): default 64 KiB, max 16 MiB; `HostImportsConfig`.
- Execution bounds (§18.2): wall-clock timeout, outer memory ceiling, fuel metering.
- Serve flow (§18.4): alloc → write request → call export → unpack/return; never decrypts.
- `jwks_fetch` via reqwest; session state; BLS attestation signing via digstore-crypto.

### 4.8 `digstore-prover` (host)
- `Prover` / `Verifier` traits; **mock** impl (default) + **risc0** impl.
- `ChainSource` trait: `get_peak()`, `verify_block(ref)`; **coinset.org** HTTP impl + deterministic mock.
- Execution proof build/verify (§13): `program_hash`, `public_input` (client nonce + ChiaBlockRef), `public_output` commitment, node BLS signature over `(proof ‖ public_input)`, nonce binding (§13.5), chain freshness window (§13.8). Hardware-attestation alternative (§13.6) behind the same trait.

### 4.9 `digstore-remote` (host)
- axum server: full §21.2 REST surface (`GET/HEAD/PUT /stores/{id}/module`, `GET /stores/{id}`, `/roots`, `POST /content`, `/proof`, `GET/POST /delta`).
- ETag = root; `If-None-Match` → 304 (§21.7). Status codes §21.8.
- Push auth (§21.6): verify publisher BLS signature over SHA-256(root) bound to store ID; optional bearer token; fast-forward-only (409 otherwise); pending-vs-served head (202, §21.4).
- Delta sync (§21.5): chunk set difference + key-table changes.
- reqwest client: clone/fetch/pull/push.

### 4.10 `digstore-cli` (host) — `digstore`
Git verbs (§20): `init`, `add`, `commit`, `status`, `log`, `diff`, `checkout`, `cat`, `remote`, `clone`, `push`, `pull`.
- **Client-side** decryption: derive decryption key (HKDF, + salt for private stores §11.4) → AES-256-GCM open → verify GCM tag → merkle verify against trusted root → optional proof verify.
- `cat <urn>` / `checkout <root>` read resources out by URN through a host instance.

---

## 5. Threat model coverage (§10)
- **Confidentiality**: URN-derived keys, client-side decrypt only.
- **Integrity**: merkle proof to trusted root (§9) + GCM tag (§11) + execution proof (§13).
- **Blindness**: retrieval/decryption keys are URN functions; provider holds hashes + ciphertext (§15).
- **Indistinguishability**: deterministic decoys + oblivious access (§14).

---

## 6. Cross-cutting engineering
- **TDD** throughout (superpowers:test-driven-development); tests before impl per unit.
- **Determinism tests**: double-compile byte equality; deterministic filler vectors.
- **Cross-impl BLS vectors**: host-sign (blst) ↔ guest-verify (bls12_381) parity fixtures.
- **ABI golden tests**: pack/unpack round-trips, error sentinels, decoy real-vs-miss shape equality.
- **Codec conformance**: Chia-streamable encode/decode round-trip + fixed byte-vector fixtures.
- **End-to-end fixtures**: compiled sample modules served through the host; full init→add→commit→cat→clone→push→pull flows.
- Pinned Rust toolchain + locked wasm32 build for reproducible guest template.

---

## 7. Out of scope (per paper)
Network distribution across many providers, external identity anchoring of the store ID, payment settlement. Single local store format + its HTTPS remote protocol only.

---

## 8. Build phases (high level — detailed plan is the companion implementation-plan doc)
1. `digstore-core` + `digstore-chunker` + `digstore-crypto` — foundation.
2. `digstore-store` — entity, generations, on-disk, staging.
3. `digstore-guest` — exports, decoys, attestation, oblivious access, JWT → wasm32.
4. `digstore-compiler` — data injection, key table, determinism, obfuscation.
5. `digstore-host` — wasmtime, imports, bounds, serve flow.
6. `digstore-cli` — git verbs, client decrypt/verify → first end-to-end.
7. `digstore-prover` — mock + ChainSource(coinset.org) + risc0.
8. `digstore-remote` — axum server + client, delta, push-auth → full end-to-end.
