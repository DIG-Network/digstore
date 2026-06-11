# Chainstate-in-WASM — Design

**Date:** 2026-06-11
**Status:** Approved (design)
**Scope:** Couple a store's on-chain anchor state into its compiled WASM module so the
module is self-describing for chain lookup, and (fast-follow) make `clone`/`pull`
verify served content against the on-chain singleton.

## Summary

Today a store's on-chain anchor state lives only in the CLI-owned `<store>/anchor.toml`
(off-module). The compiled `.dig` WASM module already embeds the `StoreId` (= the Chia
launcher id) but carries no other chain information. This feature embeds the chain
pointer **into the module's data section** so that:

1. Any app holding the module bytes can read where the store lives on-chain and look it
   up (network + launcher id + current singleton coin), with no local `anchor.toml`.
2. (Fast-follow) `clone`/`pull` can verify the served root against the on-chain singleton
   root, closing `SECURITY.md` residual #6 — the gap left by the Option-2 identity model
   (`store_id := launcher id`, §20.1 self-cert relaxed). The chain — not a self-signed
   head — becomes the authority for "the current root for this store."

Two phases, sequenced: **Phase A** (embed + read, self-describing locator) ships first;
**Phase B** (chain-verified clone/pull) is the fast-follow in the same effort.

## Background / current state

- Module data section format: `digstore_core::datasection` — a header + an id-keyed
  offset table (`SectionId: u16`, offset, len) + bodies. `DataView::section(id)` looks up
  by id and **unknown ids are ignored on parse**, so adding a new section is
  backward/forward compatible.
- Existing sections: `StoreId(1), CurrentRoot(2), RootHistory(3), PublicKey(4),
  TrustedKeys(5), Metadata(6), AuthInfo(7), KeyTable(8), ChunkPool(9), MerkleNodes(10),
  Filler(11)`. `Filler` is size-obfuscation padding and must remain the last body.
- The module is compiled at `commit` time (`digstore-cli` `store_ops::finalize_commit` →
  `compile_module` → `digstore_compiler`), NOT at `init` (init mints but compiles no
  module). `finalize_commit` runs only after a confirmed anchor.
- `digstore_compiler::data_section.rs`:
  - `encode_data_section(&DataSectionInputs)` emits the blob.
  - `swap_trusted_keys` rebuilds the blob over a FIXED `IDS` list (used when a downloaded
    module's trusted keys are swapped) — any new section must be added to that list or it
    is dropped on rebuild.
  - `verify_module_root(module, expected_store_id)` checks embedded `StoreId == expected`
    and recomputed merkle root == embedded `CurrentRoot` (the §20.1 `SHA-256(pubkey)==id`
    check was removed in Phase 5).
- `digstore-chain` provides the singleton read path: `sync_datastore(&dyn ChainReads,
  launcher_id) -> DataStore` (walks lineage over coinset), and the `ChainAnchor` trait.
- `DIGSTORE_ANCHOR_MOCK` is the env-gated mock-anchor seam used by tests (offline).

## Decisions (locked in brainstorming)

| Decision | Choice |
|----------|--------|
| Scope/sequencing | Locator first (Phase A), chain-verified clone/pull as fast-follow (Phase B). Both in scope. |
| Field set | network, launcher_id, coin_id, confirmed_height, tx_id, **and** an embedded `coinset_url` endpoint hint. |
| Endpoint hint | Embedded `coinset_url` is a **fallback hint only**; local global-config/flags override it (avoid stale-endpoint footgun). |
| Section vs reuse | A new core `SectionId::ChainState`, not an overload of `Metadata`. |
| Snapshot semantics | Per-generation: the module for root R embeds the singleton coin that committed R. First appears at the first commit. |
| `anchor.toml` | Unchanged. Local mutable working state; the module is a point-in-time snapshot. |

## Architecture

### Phase A — embed + read

**1. `digstore-core`: the `ChainState` section.**
- Add `SectionId::ChainState = 12`.
- New `pub struct ChainState { version: u8, network: String, launcher_id: Bytes32,
  coin_id: Bytes32, confirmed_height: u32, tx_id: String, coinset_url: String }` with
  `encode() -> Vec<u8>` / `decode(&[u8]) -> Result<ChainState, DecodeError>` built on the
  existing core `Encode`/`Decode` primitives (BE-length-prefixed strings, raw `Bytes32`).
  `version` allows future field growth.
- Add `pub fn read_chain_state(blob: &[u8]) -> Result<Option<ChainState>, DecodeError>`:
  `DataView::parse(blob)` → `section(ChainState)` → `decode`. Returns `Ok(None)` when the
  section is absent (older modules). This is the single reader the CLI and any external
  app use.

**2. `digstore-compiler`: emit + preserve.**
- `DataSectionInputs` gains `chain_state: Option<ChainState>`.
- `encode_data_section` appends the `ChainState` body (when `Some`) **before** the
  `Filler` body. The numeric id (12) and the encode order are independent (offset table),
  but ChainState must precede Filler so filler stays the trailing padding.
- Add `SectionId::ChainState` to the `swap_trusted_keys` rebuild `IDS` list so a
  re-encoded (downloaded, key-swapped) module preserves the chain pointer verbatim.
- `verify_module_root` is unchanged in Phase A. (`ModuleIdentity` MAY optionally surface
  the decoded `ChainState` for callers; not required.)

**3. `digstore-cli`: write at commit.**
- `finalize_commit` (post-confirmation) has launcher (= `store_id`), the new `coin_id`,
  `confirmed_height`, `network` ("mainnet"), and `coinset_url` from `GlobalConfig`. It
  builds a `ChainState` and threads it into `compile_module` → `DataSectionInputs`.
- `tx_id`: best-effort. The current `MintOutcome`/`UpdateOutcome` do not carry a spend/tx
  id, so `tx_id` is populated from `AnchorState.last_tx_id` when set, else empty. Wiring a
  real tx id through the chain outcomes is a minor follow-up, out of scope here.

**4. `digstore-cli`: read / surface.**
- `digstore anchor status` additionally decodes the current module's embedded
  `ChainState` (via `read_chain_state`) and displays it next to `anchor.toml` (human +
  `--json`).
- New `digstore anchor inspect <module.dig>`: decode and print the chain pointer of ANY
  module file (the "app reads the wasm" path), human + `--json`. Read-only, no chain call.
- The effective `coinset_url` for live calls is global-config/flag first, embedded hint as
  fallback only.

### Phase B — chain-verified clone/pull (fast-follow)

- On `clone`/`pull`, after `verify_module_root` passes, read `launcher_id` (= verified
  `StoreId`) + `network` from the module. Use `digstore-chain` (`sync_datastore`, with the
  embedded `coin_id` as a fast-path start point, falling back to a full lineage walk) to
  fetch the singleton's **current on-chain root**, and require the module's embedded
  `CurrentRoot` == that on-chain root (and the served root matches, as today). **Fail
  closed** on mismatch or unreachable chain (with a clear error).
- This upgrades the trust model from "self-consistent module + self-signed head" to
  "the chain authorizes the current root", closing `SECURITY.md` residual #6. Update
  SECURITY.md accordingly.
- Offline testing: extend the `DIGSTORE_ANCHOR_MOCK` seam into the remote/verification
  path so the mock chain returns a deterministic current root for a launcher id; tests
  cover verify-pass and fail-closed-on-mismatch without touching mainnet.
- Coinset endpoint resolution for the verification call: global config/flag, falling back
  to the embedded hint, then the default.

## Data flow

```
commit (confirmed)  ── finalize_commit builds ChainState{network,launcher,coin_id,height,tx_id,coinset_url}
                       └─ compile_module → DataSectionInputs.chain_state → encode_data_section
                          → module .dig embeds SectionId::ChainState

app / CLI            ── read_chain_state(module bytes) → ChainState  (no chain call, no guest)
                       └─ `anchor inspect` / `anchor status`

clone / pull (B)     ── verify_module_root → launcher_id, CurrentRoot
                       └─ digstore-chain sync_datastore(launcher) → on-chain root
                          └─ require embedded CurrentRoot == on-chain root  (fail closed)
```

## Error handling

- `read_chain_state`: absent section → `Ok(None)` (backward compat); malformed →
  `DecodeError` surfaced as a CLI error on `inspect`/`status` (but a malformed ChainState
  must NOT break `verify_module_root` in Phase A — the verifier does not read it).
- Phase B: chain unreachable → fail closed with a `Chain`/`VerificationFailed` error and a
  clear hint (do not silently fall back to the self-signed head). Root mismatch →
  `VerificationFailed`.

## Components (isolation)

- `digstore-core::datasection`: `ChainState` type + `encode/decode` + `read_chain_state` +
  the new `SectionId`. One responsibility: the on-disk/in-module chainstate format.
- `digstore-compiler`: carry `ChainState` through `DataSectionInputs`/`encode`/swap. No
  chain logic.
- `digstore-cli`: populate at commit; `anchor status`/`anchor inspect` readers; Phase B
  wires `digstore-chain` into clone/pull verification.
- `digstore-chain`: unchanged for embedding; Phase B reuses `sync_datastore`.

## Testing

**Phase A**
- core: `ChainState` encode/decode round-trip; `read_chain_state` returns `None` for a
  blob without the section and `Some` for one with it; malformed body → error.
- compiler: `encode_data_section` with `chain_state: Some(..)` emits a decodable section
  and keeps `Filler` last; `swap_trusted_keys` preserves `ChainState`; a module compiled
  WITHOUT chain_state still `verify_module_root`s (backward compat).
- cli: a mock-anchored `commit` produces a module whose `read_chain_state` returns the
  expected launcher/coin/network; `anchor status` and `anchor inspect <module>` show it
  (human + `--json`); all via the `DIGSTORE_ANCHOR_MOCK` seam, offline.

**Phase B**
- mock-chain clone/pull: verify-pass when the mock singleton root equals the module's
  `CurrentRoot`; fail-closed when they differ; fail-closed when the chain is unreachable.
- existing clone/pull/tamper/revoke suites stay green (the new check is additive and runs
  after the existing ones).

## Out of scope

- Wiring a real spend/tx id through `MintOutcome`/`UpdateOutcome` (best-effort `tx_id` for
  now).
- Re-introducing any `SHA-256(pubkey)==store_id` binding (Option-2 identity stands).
- Multi-network / testnet (network is "mainnet").

## Security notes

- Phase A is informational and additive: a malformed/forged `ChainState` cannot weaken
  `verify_module_root` (which never reads it) and cannot be mistaken for verification.
- Phase B is the security upgrade: it makes the chain the authority for the current root,
  closing residual #6. It must fail closed (never fall back to trusting the served head on
  a chain error) and is gated behind the offline mock seam for CI. SECURITY.md is updated
  when Phase B lands.
