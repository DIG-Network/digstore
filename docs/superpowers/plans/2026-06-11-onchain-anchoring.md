# Onchain Anchoring Implementation Plan (Subsystem 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Anchor every digstore store on Chia mainnet via coinset.org: `init` mints an empty store singleton (its launcher id becomes the `store_id`), and every `commit` pushes the new root on-chain via a singleton `update` that blocks until confirmed before finalizing locally. No P2P peer, no TLS cert — all broadcast and reads go through coinset HTTP.

**Architecture:** Extend the existing `digstore-chain` crate (seed management already shipped) with `coinset.rs` (HTTP client), `keys.rs` (mnemonic→synthetic key→puzzle hash), `singleton.rs` (build/sign mint+update over datalayer-driver low-level fns + reconstruct `DataStore` from coinset), and `anchor.rs` (the `ChainAnchor` trait + impl). `digstore-cli` gates `init`/`commit` on seed+funds, adds confirmation UX, and `anchor`/`anchor status` commands. Commands that touch the chain run async via a `tokio` runtime bridged from the sync dispatch.

**Tech stack:** Rust. `datalayer-driver 3.0` (Peer-free builders: `mint_store`, `update_store_metadata`, `select_coins`, `sign_coin_spends`, `spend_bundle_to_hex`, `master_secret_key_to_wallet_synthetic_secret_key`, `synthetic_key_to_puzzle_hash`, `get_mainnet_genesis_challenge`), `chia-protocol`/`chia-sdk-driver` types it re-exports (`Coin`, `CoinSpend`, `SpendBundle`, `DataStore`, `Proof`), `reqwest` (coinset HTTP), `serde_json`, `tokio`. Seed crypto reuses the shipped `digstore-chain::seed`/`unlock`.

**Spec:** `docs/superpowers/specs/2026-06-11-onchain-anchoring-design.md` (see the "Verification spike results (2026-06-11)" section — it supersedes the earlier "wrap dig-store-coin" assumption).

---

## Why this plan leads with a prototype

The verification spike (in the spec) pinned the build/sign + key-derivation API and confirmed coinset exposes `push_tx` / `get_coin_record_by_name` / `get_coin_records_by_puzzle_hash`. It also established that the **`update`/singleton-sync path is not derivable from docs**:
- `update_store_metadata` needs a full `DataStore { coin, proof: Proof, info: DataStoreInfo }` for the current singleton, normally produced by the Peer-based `sync_store_from_launcher_id`. Over coinset we must reconstruct it from coin records + puzzle-and-solution reveals.
- `SuccessResponse`'s exact fields are unconfirmed (re-exported type; docs.rs 404).
- `DataStoreInnerSpend` construction (owner authorization for the update) is unconfirmed.

**Phase 0 is a throwaway, testnet-only prototype** that resolves these against the real compiler and a live chain before any production code is committed. Phases 1+ build production code on what Phase 0 pins. Do NOT skip Phase 0; do NOT write the `singleton.rs` update path before the prototype proves it.

---

## File structure

**Extend `crates/digstore-chain/`:**
- `Cargo.toml` — add `datalayer-driver = "3"`, `chia-protocol`, `reqwest` (workspace), `tokio` (workspace). (`serde_json` already present.)
- `src/lib.rs` — add `pub mod coinset; pub mod keys; pub mod singleton; pub mod anchor;`
- `src/coinset.rs` — coinset.org HTTP client (push_tx, coin-record reads, blockchain state) behind a `CoinsetApi` trait for stub testing.
- `src/keys.rs` — mnemonic → master `SecretKey` → wallet synthetic secret key → owner puzzle hash.
- `src/singleton.rs` — build/sign mint + update spends; reconstruct `DataStore` from coinset. (Shape finalized by Phase 0.)
- `src/anchor.rs` — `ChainAnchor` trait + `CoinsetAnchor` impl + mock; `MintOutcome`/`UpdateOutcome`/`AnchorStatus`/`Balances`.
- `src/error.rs` — add anchoring error variants.

**Extend `crates/digstore-cli/`:**
- `Cargo.toml` — add `tokio` (already present), nothing else new.
- `src/error.rs` — `InsufficientFunds`, `PeerUnreachable`→`Chain`, `ConfirmTimeout`, `MintFailed`, `UpdateFailed` variants + hints.
- `src/runtime.rs` — `block_on` helper (one tokio runtime) since dispatch is sync.
- `src/commands/anchor.rs` — `digstore anchor` / `anchor status`.
- `src/commands/init.rs`, `src/commands/commit.rs` — prereq-gate on seed+unlock, then anchor.
- `src/cli.rs` — `Anchor(AnchorArgs)` + `--wait-timeout` on init/commit.
- `src/ops/store_ops.rs` — write/read the `[anchor]` table in `config.toml`; commit finalize gated on confirmed anchor.

**Throwaway (Phase 0, NOT committed to src):**
- `crates/digstore-chain/examples/anchor_prototype.rs` — gated example proving mint+update over coinset on testnet11.

---

## Phase 0 — Prototype spike (testnet11, throwaway)

> Goal: pin `mint_store`/`update_store_metadata`/`SuccessResponse`/`DataStore`-reconstruction/`DataStoreInnerSpend` against the real compiler and a live chain. Runs on **testnet11** (free coins) even though production targets mainnet — never spend mainnet XCH during prototyping. Output is knowledge + confirmed signatures recorded in this plan, not production code.

### Task 0.1: Add chain deps + confirm the crate compiles against them

**Files:** `crates/digstore-chain/Cargo.toml`

- [ ] **Step 1:** Add to `[dependencies]`:
```toml
datalayer-driver = "3"
chia-protocol = "0.26"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros"] }
```
- [ ] **Step 2:** Run `cargo build -p digstore-chain`. Expected: PASS (deps resolve + compile). These crates pull native Chia deps; if the build fails to resolve/compile on this toolchain, STOP and report — that is a go/no-go signal for the whole subsystem.
- [ ] **Step 3:** Commit `chore(chain): add datalayer-driver + reqwest deps for anchoring`.

### Task 0.2: Prototype example — derive keys, fetch coins, mint on testnet, push_tx via coinset

**Files:** `crates/digstore-chain/examples/anchor_prototype.rs` (throwaway; gated behind `--features prototype` or just an example run manually)

- [ ] **Step 1:** Write an example `main` (async, `#[tokio::main]`) that, given a testnet mnemonic + `DIGSTORE_PROTO_PHRASE` env:
  1. `bip39` mnemonic → seed → `chia-protocol`/`datalayer-driver` master `SecretKey` (pin the exact derivation: `bip39` seed → BLS master key — confirm whether dig-wallet/datalayer-driver expose a `master_secret_key_from_seed`-style fn, else use `chia-bls` `SecretKey::from_seed`).
  2. `master_secret_key_to_wallet_synthetic_secret_key(&master)` → synthetic SK; `secret_key_to_public_key` → synthetic PK; `synthetic_key_to_puzzle_hash(&pk)` → owner puzzle hash.
  3. POST `https://api-testnet.coinset.org/get_coin_records_by_puzzle_hash` with the owner puzzle hash → parse unspent `Coin`s. **Record the exact JSON response shape** (coin fields, hex prefixes) in this plan.
  4. `select_coins(&coins, fee)` for a small fee.
  5. `mint_store(synthetic_pk, selected_coins, root_hash = [0u8;32], None, None, None, None, owner_puzzle_hash, vec![], fee)` → `SuccessResponse`. **Record `SuccessResponse`'s real fields** (coin_spends? new `DataStore`? launcher id?) by inspecting the compiler/type.
  6. Build a `SpendBundle` from the response's coin spends + `sign_coin_spends(&spends, &[synthetic_sk], for_testnet = true)`; `spend_bundle_to_hex`.
  7. POST `https://api-testnet.coinset.org/push_tx` with the spend bundle JSON → record the ack response shape.
  8. Poll `get_coin_record_by_name` for the launcher coin until confirmed; print the launcher id (= store_id).
- [ ] **Step 2:** Run it manually on testnet11 with a funded test wallet. Iterate until a store mints and confirms.
- [ ] **Step 3:** **Record in this plan**, under "Phase 0 findings" below: exact `SuccessResponse` fields; the coinset request/response JSON for `get_coin_records_by_puzzle_hash`, `push_tx`, `get_coin_record_by_name`; the precise key-derivation chain; how the launcher id is obtained from the mint response.

### Task 0.3: Prototype the `update` path — reconstruct DataStore from coinset, update root

- [ ] **Step 1:** Extend the example: given the launcher id from 0.2, reconstruct the current `DataStore` from coinset reads alone:
  1. From the launcher coin, follow the singleton lineage via `get_coin_record_by_name` / `get_coin_records_by_parent_ids` + `get_puzzle_and_solution` to find the latest unspent singleton coin and build its `Proof` (lineage proof) and `DataStoreInfo`/metadata.
  2. Construct `DataStoreInnerSpend` (owner authorization) — **record how it is built** (synthetic key spend).
  3. `update_store_metadata(store, new_root_hash, None, None, None, None, inner_spend_info)` → `SuccessResponse`; sign; `push_tx`; poll to confirmed.
- [ ] **Step 2:** Run on testnet until an `update` confirms and a re-read shows the new root.
- [ ] **Step 3:** **Record in this plan** the working `DataStore`-reconstruction algorithm over coinset (the single hardest piece), the `DataStoreInnerSpend` construction, and `update_store_metadata`'s real argument types. If reconstruction proves infeasible over coinset reads alone, STOP and escalate — this is the go/no-go gate for the commit/update feature.

### Task 0.4: Delete the prototype, write up findings

- [ ] **Step 1:** Append a "## Phase 0 findings" section to this plan with all recorded signatures/shapes/algorithms.
- [ ] **Step 2:** Remove `examples/anchor_prototype.rs`. Commit `docs(plan): record anchoring Phase-0 prototype findings`.

> **Gate:** Phases 1+ may only be written/executed after Phase 0 findings are recorded. The concrete code in later tasks that touches `SuccessResponse`/`DataStore`/`DataStoreInnerSpend` MUST be reconciled against the findings before implementation.

---

## Phase 1 — Coinset HTTP client (`coinset.rs`)

Fully specifiable now (REST shapes confirmed in Phase 0; the trait + tests do not depend on Chia internals).

### Task 1.1: `CoinsetApi` trait + `CoinsetClient` (reqwest) + stub tests

**Files:** `crates/digstore-chain/src/coinset.rs`, `crates/digstore-chain/Cargo.toml` (add `wiremock` dev-dep)

- [ ] **Step 1 (test first):** Add `wiremock = "0.6"` and `tokio` test feature to `[dev-dependencies]`. Write a test that starts a `wiremock` server returning a canned `push_tx` success body and asserts `CoinsetClient::push_tx` parses `{ "success": true, "status": "SUCCESS" }` into `Ok(TxAck { success: true, .. })`, and a `{ "success": false, "error": "..." }` body into an error.
- [ ] **Step 2 (test):** `get_coin_records_by_puzzle_hash` returns parsed `Vec<CoinRecord>` from a canned body (use the exact shape recorded in Phase 0).
- [ ] **Step 3 (test):** `get_coin_record_by_name` returns `Option<CoinRecord>` (None when `coin_record: null`).
- [ ] **Step 4 (implement):** Define:
```rust
#[async_trait::async_trait]
pub trait CoinsetApi: Send + Sync {
    async fn push_tx(&self, spend_bundle_json: serde_json::Value) -> Result<TxAck>;
    async fn coin_records_by_puzzle_hash(&self, puzzle_hash_hex: &str, include_spent: bool) -> Result<Vec<CoinRecord>>;
    async fn coin_record_by_name(&self, name_hex: &str) -> Result<Option<CoinRecord>>;
    async fn coin_records_by_parent_ids(&self, parent_ids_hex: &[String], include_spent: bool) -> Result<Vec<CoinRecord>>;
    async fn puzzle_and_solution(&self, coin_id_hex: &str, height: u32) -> Result<PuzzleAndSolution>;
    async fn blockchain_state(&self) -> Result<BlockchainState>; // peak height
}
```
plus `CoinsetClient { base_url: String, http: reqwest::Client }` implementing it via POST `{base_url}/{endpoint}`, and the `CoinRecord`/`TxAck`/`PuzzleAndSolution`/`BlockchainState` structs (fields per Phase 0). `base_url` defaults to `https://api.coinset.org`.
- [ ] **Step 5:** `cargo test -p digstore-chain coinset::`. Commit `feat(chain): coinset.org HTTP client`.

---

## Phase 2 — Key derivation (`keys.rs`)

### Task 2.1: mnemonic → synthetic SK/PK → owner puzzle hash, with vectors

**Files:** `crates/digstore-chain/src/keys.rs`

- [ ] **Step 1 (test):** Using the derivation chain pinned in Phase 0, add a test that a known testnet mnemonic produces the expected owner puzzle hash (capture the expected value from the Phase 0 prototype run as the fixture).
- [ ] **Step 2 (implement):** `pub struct WalletKeys { pub synthetic_sk: SecretKey, pub synthetic_pk: PublicKey, pub owner_puzzle_hash: Bytes32 }` and `pub fn derive_wallet_keys(mnemonic: &str) -> Result<WalletKeys>` chaining bip39 seed → master SK → `master_secret_key_to_wallet_synthetic_secret_key` → `secret_key_to_public_key` → `synthetic_key_to_puzzle_hash`. Wrap secret material so it does not linger (reuse the `Zeroizing` discipline from `seed.rs`).
- [ ] **Step 3:** `cargo test -p digstore-chain keys::`. Commit `feat(chain): wallet key derivation from mnemonic`.

---

## Phase 3 — Singleton build/sign + sync (`singleton.rs`)

> Each task here MUST be reconciled with Phase 0 findings before coding. Signatures below are the pinned datalayer-driver API; the `SuccessResponse`/`DataStore` field access is filled in from Phase 0.

### Task 3.1: Build + sign the mint spend
- [ ] Test (mock coinset): given canned unspent coins, `build_mint(keys, root = EMPTY, fee)` selects coins, calls `mint_store(...)`, signs with `sign_coin_spends(&spends, &[synthetic_sk], false)`, returns `{ spend_bundle_json, launcher_id }`. Assert the launcher id matches the value derivable from the response.
- [ ] Implement per Phase 0 findings. Commit.

### Task 3.2: Reconstruct `DataStore` from coinset (the crux)
- [ ] Test (mock coinset with canned lineage): `sync_datastore(coinset, launcher_id)` returns the current unspent singleton `DataStore` (coin + Proof + info) — fixtures captured from the Phase 0 testnet run.
- [ ] Implement the lineage-walk recorded in Phase 0. Commit. (If Phase 0 found this infeasible, this task does not exist and the plan stops — escalated at 0.3.)

### Task 3.3: Build + sign the update spend
- [ ] Test (mock): `build_update(keys, datastore, new_root, fee)` → signed `{ spend_bundle_json, new_coin_id }` via `update_store_metadata(...)` + sign. 
- [ ] Implement per Phase 0. Commit.

---

## Phase 4 — `ChainAnchor` (`anchor.rs`)

### Task 4.1: trait + types + mock
- [ ] Define (matches the spec):
```rust
#[async_trait::async_trait]
pub trait ChainAnchor: Send + Sync {
    async fn balances(&self, keys: &WalletKeys) -> Result<Balances>;
    async fn mint_empty_store(&self, keys: &WalletKeys, fee: u64) -> Result<MintOutcome>;
    async fn update_root(&self, launcher_id: Bytes32, new_root: Bytes32, keys: &WalletKeys, fee: u64) -> Result<UpdateOutcome>;
    async fn confirm(&self, coin_id: Bytes32, timeout_secs: u64) -> Result<ConfirmState>;
}
```
with `MintOutcome { launcher_id, coin_id, tx_id }`, `UpdateOutcome { new_coin_id, tx_id }`, `Balances { xch: u64 }`, `ConfirmState { Confirmed { height: u32 }, Pending }`. Mock impl for CLI tests.
- [ ] Test the mock; commit.

### Task 4.2: `CoinsetAnchor` impl + confirmation polling
- [ ] Implement `CoinsetAnchor { coinset: Arc<dyn CoinsetApi> }`: `balances` sums unspent coins by puzzle hash; `mint_empty_store` = build_mint→push_tx→return ids; `update_root` = sync_datastore→build_update→push_tx; `confirm` polls `coin_record_by_name` + `blockchain_state` peak until the coin is confirmed/`spent` to target depth or timeout. Tests against mock coinset (canned mempool→confirmed progression). Commit.

---

## Phase 5 — CLI integration

### Task 5.1: error variants + async bridge
- [ ] Add `CliError::{InsufficientFunds { need: u64, have: u64, address: String }, Chain(String), ConfirmTimeout, MintFailed(String), UpdateFailed(String)}` + hints (fund address, "check coinset/network", "run `digstore anchor status`"). Map `ChainError` chain variants through. Add `crates/digstore-cli/src/runtime.rs` with `pub fn block_on<F: Future>(f: F) -> F::Output` using a cached `tokio` runtime. Commit.

### Task 5.2: `init` mints (hard gate)
- [ ] Gate: resolve seed (exists→unlock via the seed-mgmt `resolve_passphrase`; else `NoSeed`). Preflight `balances`; if short → `InsufficientFunds` with the receive address. `mint_empty_store` → on submit, write local store keyed on `launcher_id` as `store_id` + `[anchor] status=pending`; wait for confirm (Task 5.4 UX); flip to `confirmed`. On pre-submit failure → roll back scaffold; on post-submit timeout → keep store `pending`, exit non-zero. Tests with mock anchor. Commit.

### Task 5.3: `commit` updates (hard gate, blocks until confirmed)
- [ ] After staging→new root: `update_root` → block until confirmed → only then finalize the generation (advance `roots.log`). On failure/timeout → abort, roots.log untouched. Idempotency: if an `update` tx for this root is already pending, reuse it. Tests with mock anchor. Commit.

### Task 5.4: confirmation UX + `anchor`/`anchor status` + `[anchor]` config
- [ ] Staged human-friendly progress (submitted→mempool→confirming N/M→confirmed) with `--wait-timeout` (default 300s) and `--json` structured states. `digstore anchor` resumes a `pending` store; `digstore anchor status` queries coinset for the active store. Read/write the `[anchor]` table in `config.toml` (`store_id`/`coin_id`/`status`/`last_root`/`last_tx_id`/`confirmed_height`). Tests. Commit.

### Task 5.5: docs + full verification
- [ ] README: anchoring section (init mints, commit anchors, funding, mainnet/coinset, `anchor status`). Run `cargo test -p digstore-chain -p digstore-cli`. Optionally a gated `DIGSTORE_E2E` testnet end-to-end test (never in CI). Commit.

---

## Self-Review

**Spec coverage:** init-mints / launcher=store_id (5.2), commit-anchors-blocks-until-confirmed (5.3), mainnet+coinset transport (Phases 1/3/4), seed+funds prereq gating + guidance (5.1/5.2), `[anchor]` data model (5.4), confirmation UX (5.4), `anchor`/`anchor status` (5.4), hard-gate failure semantics (5.2/5.3), testable via mock chain + coinset stub (all phases). ✔

**Honesty about unknowns:** Phase 0 is an explicit, compile-and-testnet-verified prototype that resolves `SuccessResponse`, `DataStore` reconstruction over coinset, and `DataStoreInnerSpend` before any production code touches them. Phase 3 tasks are gated on Phase 0 findings rather than fabricating Chia-internal field access. The `DataStore`-over-coinset reconstruction (Task 0.3/3.2) is the single go/no-go risk and is flagged as an escalation point.

**Placeholder note:** Phases 1, 2, 4, 5 contain concrete real code (coinset client, key derivation, trait/mock, CLI gating) testable without mainnet. Phase 3's exact `SuccessResponse`/`DataStore` field access is intentionally deferred to Phase 0 findings — this is a verification gate, not a hand-wave: the datalayer-driver function signatures are pinned, only the response-struct internals await the prototype.

**Cost warning:** Phase 0 and any e2e run on **testnet11** (free). Production `init`/`commit` spend **real mainnet XCH** on every operation — surfaced to users via the preflight balance check and confirmation UX.
