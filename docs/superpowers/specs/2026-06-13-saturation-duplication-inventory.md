# Saturation duplication inventory — all layers

**Date:** 2026-06-13
**Method:** parallel read-only audit across digstore_wasm (Rust), hub.dig.net (Rust backend), hub.dig.net/apps/web (JS).
**Goal:** find ALL logic/constants duplicated or mirrored across layers, and decide the single home for each.

Legend — Risk: **SKEW** (a silent mismatch breaks content/auth), **DRY** (maintenance only), **SEC** (domain-separation/security hygiene).

## A. Inside digstore_wasm

| Item | Copies | Home | Risk |
|------|--------|------|------|
| `CHAIN = "chia"` | dig-client-wasm:41, digstore-cli ×6 (checkout, client_crypto, store_ops) | `digstore-core` const | SKEW |
| `DEFAULT_RESOURCE_KEY = "index.html"` | dig-client-wasm:44, digstore-cli store_ops:1184 | `digstore-core` const | SKEW |
| BLS DSTs `PUSH/NODE/TOMB/REQ_DST` | only digstore-crypto/bls.rs (152/154/160/167) | move to `digstore-core` (re-export from crypto) | SKEW |
| **`NODE_DST` == `NODE_TAG`** | bls.rs:154 `b"digstore:node:v1"` collides with merkle.rs:39 `b"digstore:node:v1"` | give BLS node-signing a DISTINCT tag (e.g. `digstore:node-sig:v1`) | **SEC** |
| HKDF domains | now in digstore-core/crypto.rs (private) | export `pub` so a verifier can reference | DRY |
| AES/HKDF crypto + `resource_leaf` | **DONE (SP1)** — now single-source in digstore-core | — | ✓ |
| `ATTEST_DST`, `LEAF_TAG`, `NODE_TAG` | already single-source in core | — | ✓ |
| signing-message builders | single-source in digstore-crypto/bls.rs (good) | — | ✓ |

## B. hub.dig.net (Rust) reproducing digstore contract

| Item | hub location | Should be | Risk |
|------|-------------|-----------|------|
| **`dighub_core::Urn`** (canonical + retrieval_key + retrieval_key_hex) | packages/dighub-core/src/urn.rs:25-77 | re-export `digstore_core::Urn` (hub already deps digstore-core) | SKEW |
| `CHAIN_ID = "chia"` | dighub-core/constants.rs:6 + hardcoded `"chia"` in retrieval:926 | from digstore-core | SKEW |
| retrieval_key via `seed_from_text` + inline URN | retrieval/bootstrap.rs:924-929,1641-1647 | `Urn::retrieval_key()` | SKEW |
| `parse_retrieval_key` / `decode_retrieval_key` | retrieval:1631-1639 AND api/store.rs:1481-1489 | one `dighub_core` helper | DRY |
| manifest path `".well-known/dig/manifest.json"` | api/store.rs:2440 AND retrieval:926 | `dighub_core::MANIFEST_RESOURCE_PATH` | SKEW |
| PoW domain `"dighub:proof-pow:v1:"` | api/router.rs:1723 (+ dto.rs:867 comment) | `dighub_core::PROOF_POW_DOMAIN` | DRY |
| request-auth method tag + verify (mirrors digstore-remote "byte-for-byte") | retrieval:1677-1746 | shared with digstore-remote | SKEW |
| Good (already centralized): `HEADER_INCLUSION_PROOF`, `PROOF_POW_BITS`, BLS verify delegated to `digstore_crypto::verify_request` | — | — | ✓ |

## C. Frontend JS

| Item | Copies | Should be | Risk |
|------|--------|-----------|------|
| retrieval_key in pure JS (`sha256(urn)`) | loader.js:81-90 (dig-client.js uses WASM) | route loader.js through WASM `dig.retrievalKey` | SKEW |
| `b64ToBytes` + 3-MiB chunk loop | loader.js:15-62 AND dig-client.js:75-132 | shared `lib/rpc-client.js` | DRY |
| sha256→hex helper | store-key.js, flows.js, dig-client.js, loader.js, spend-convert.js | shared `lib/crypto-utils.js` | DRY |
| RPC endpoint + jsonrpc envelope | loader.js:12,37 AND dig-client.js:60,90 | shared `lib/config.js` + rpc-client | DRY |
| store-id regex, chain tag, default resource | loader.js + parseUrn | shared `lib/config.js` | DRY |
| Good: `parseUrn` single-source; verify/decrypt WASM-only | — | — | ✓ |

## Revised decomposition (post-saturation)

- **SP1 — Contract crypto + `resource_leaf` → digstore-core.** ✅ DONE + deployed (byte-identical).
- **SP2 — Constants + DST consolidation + domain-collision fix + format-version tag.** Move `CHAIN`,
  `DEFAULT_RESOURCE_KEY`, the BLS DSTs into digstore-core; fix `NODE_DST`/`NODE_TAG` collision (distinct
  BLS node DST); export HKDF domains. Add a single `CONTRACT_VERSION` embedded in the proof/module so a
  producer/verifier mismatch fails LOUD (the skew that started this). Wire-affecting → version bump +
  migration note. Update digstore-cli + dig-client-wasm to import the centralized consts.
- **SP3 — hub Rust ↔ digstore contract unification.** `dighub_core` re-exports `digstore_core::Urn`
  (delete the duplicate); retrieval uses `Urn::retrieval_key` (drop `seed_from_text`); add
  `MANIFEST_RESOURCE_PATH` + `PROOF_POW_DOMAIN`; one shared `parse_retrieval_key`; share the request-auth
  message with digstore-remote.
- **SP4 — Frontend JS convergence.** loader.js derives the retrieval key via WASM; shared
  `lib/{crypto-utils,rpc-client,config}.js` consumed by both loader.js and dig-client.js.
- **SP5 — CI version-lock + producer artifact.** Build+publish BOTH the compile binary and the verifier
  wasm in CI from ONE pinned digstore rev; pin every consumer to it; assert the CONTRACT_VERSION matches
  across producer/host/verifier. This is the operational fix for the original out-of-band drift.

Single-source target: `digstore-core` owns the entire wire/crypto/addressing/version contract; every
layer (producer, host, verifier-wasm, hub Rust via dep, frontend via the wasm) consumes it — no copies.
