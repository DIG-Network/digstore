# DIG CAT Payment + Balance Gating — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Embed a DIG CAT payment in every store mint/update spend bundle (init → 100 DIG, commit → 10 DIG, memo = store id), add a `digstore balance` (XCH + DIG) command, and block init/commit before any spend when XCH or DIG is short.

**Architecture:** New `digstore-chain/src/cat.rs` finds/values/spends DIG CAT coins over coinset (lineage reconstructed from coinset reads, mirroring the singleton sync). `anchor.rs` concatenates the DIG-payment coin spends into the singleton mint/update bundle and signs them together with the same synthetic key (atomic). The CLI adds `balance` and gates `init`/`commit` on a balance preflight.

**Tech Stack:** Rust. `chia` (`chia::puzzles::cat::CatArgs`, `Memos`), `chia-wallet-sdk` (`driver::{Cat, SpendContext, Spends, Action, SpendWithConditions, StandardLayer}`, `Cat::parse_children`), `datalayer-driver` (singleton + sign), `chia-sdk-coinset` (reads/push). Spec: `docs/superpowers/specs/2026-06-11-dig-cat-payment-design.md`. Reference (P2P, NOT used directly): `c:\Users\micha\workspace\dig_network\DataLayer-Driver` (`src/wallet.rs` `DIG_ASSET_ID`/`send_xch`; `src/dig_coin.rs` `DigCoin`; `src/dig_collateral_coin.rs` CAT-spend pattern via `Spends`/`Action`).

**Conventions (all tasks):** TDD. Conventional commits, SSH-signed, **NO `Co-Authored-By` trailer**. If a build panics about a missing `digstore_guest.wasm`, run `cargo build -p digstore-guest --target wasm32-unknown-unknown --release` first. Offline tests use the `DIGSTORE_ANCHOR_MOCK` seam; never spend real funds except the explicitly-gated Phase 0 / Phase 5 live steps, and only after `dig_balance` confirms funds + the user approves.

**Constants (locked, from the spec):**
- `DIG_ASSET_ID = a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81`
- Treasury recipient address `xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y`
- 3 decimals (1 DIG = 1000 base units). init = 100_000 base; commit = 10_000 base.
- memo (init AND commit) = store id (launcher id), raw 32 bytes, in addition to the recipient hint.

---

## File structure

- `crates/digstore-chain/Cargo.toml` — **modify**: ensure `chia` (cat puzzles) + `chia-wallet-sdk` driver are available (chia-wallet-sdk already used by `singleton.rs`).
- `crates/digstore-chain/src/dig.rs` — **create**: DIG constants (`DIG_ASSET_ID`, `TREASURY_ADDRESS`, `TREASURY_INNER_PUZZLE_HASH`, `INIT_DIG`, `COMMIT_DIG`, `DIG_DECIMALS`) + amount/format helpers.
- `crates/digstore-chain/src/cat.rs` — **create**: `dig_cat_puzzle_hash`, `dig_balance`, DIG CAT-coin reconstruction over coinset, `build_dig_payment`.
- `crates/digstore-chain/src/anchor.rs` — **modify**: append the DIG payment to mint/update bundles; expose `dig_balance` on the anchor.
- `crates/digstore-chain/src/lib.rs` — **modify**: `pub mod dig; pub mod cat;`.
- `crates/digstore-chain/examples/dig_payment_prototype.rs` — **create then delete** (Phase 0-DIG).
- `crates/digstore-cli/src/commands/balance.rs` — **create**: `digstore balance`.
- `crates/digstore-cli/src/cli.rs`, `src/commands/mod.rs` — **modify**: wire `Balance`.
- `crates/digstore-cli/src/error.rs` — **modify**: `InsufficientFunds { need, have, address, asset }`.
- `crates/digstore-cli/src/commands/init.rs`, `commit.rs` — **modify**: DIG+XCH preflight; (the DIG payment itself is added inside `digstore-chain` anchor, so init/commit only gate).

---

## Phase 0-DIG — prototype spike (mainnet, throwaway)

> Goal: pin the exact `chia-wallet-sdk` CAT API for (a) reconstructing the wallet's DIG CAT coins + lineage proofs over **coinset** (no Peer), (b) building a CAT spend that sends a DIG amount to the treasury with memos `[recipient_hint, store_id]` + change, and (c) combining those coin spends with a `mint_store` spend into ONE signed `SpendBundle` pushed via coinset. Output = a recorded recipe + confirmed mainnet tx, not production code.

### Task 0.1: deps + DIG constants + DIG balance read (read-only)

**Files:** `crates/digstore-chain/Cargo.toml`, `crates/digstore-chain/src/dig.rs`, `src/lib.rs`, `src/cat.rs` (stub for `dig_cat_puzzle_hash` + `dig_balance`)

- [ ] **Step 1:** Confirm `chia` exposes `chia::puzzles::cat::CatArgs` and `chia-wallet-sdk` exposes `driver::Cat` in this workspace. Run `cargo tree -p digstore-chain | grep -E 'chia-wallet-sdk|^chia '`. If `chia` is not a direct dep, add `chia = "0.26"` (match the `chia-protocol` minor already in use) to `[dependencies]`. Build: `cargo build -p digstore-chain`. Expected PASS.

- [ ] **Step 2:** Create `src/dig.rs`:

```rust
//! DIG token (CAT) constants + amount helpers. The DIG treasury is paid on every
//! store mint/update (see the DIG-CAT-payment design).
use chia_protocol::Bytes32;

/// DIG CAT asset id (mainnet). Matches DataLayer-Driver `DIG_ASSET_ID`.
pub const DIG_ASSET_ID: Bytes32 = Bytes32::new(hex_literal::hex!(
    "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81"
));

/// DIG treasury recipient (bech32 `xch1…`). The DIG payment is sent to this
/// address's CAT puzzle hash.
pub const TREASURY_ADDRESS: &str =
    "xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y";

/// DIG has 3 decimals: 1 DIG = 1000 base units.
pub const DIG_DECIMALS: u32 = 3;
/// Base units charged to mint a store (`init`): 100 DIG.
pub const INIT_DIG: u64 = 100_000;
/// Base units charged per root update (`commit`): 10 DIG.
pub const COMMIT_DIG: u64 = 10_000;

/// The treasury's inner (standard) puzzle hash, decoded from `TREASURY_ADDRESS`.
pub fn treasury_inner_puzzle_hash() -> Bytes32 {
    // `datalayer_driver::address_to_puzzle_hash` (re-exported) inverts
    // `puzzle_hash_to_address`. (Confirm the exact fn name in Step 3.)
    datalayer_driver::address_to_puzzle_hash(TREASURY_ADDRESS, "xch")
        .expect("TREASURY_ADDRESS is a valid xch address")
}

/// Format base units as a human DIG string (÷1000, 3 dp).
pub fn format_dig(base_units: u64) -> String {
    format!("{}.{:03}", base_units / 1000, base_units % 1000)
}
```

  Add `hex-literal` to deps if not present (DataLayer-Driver uses `hex!`; check `Cargo.toml`). In `src/lib.rs` add `pub mod dig;`. **Confirm** `datalayer_driver::address_to_puzzle_hash` exists (it re-exports the bech32 helpers; `puzzle_hash_to_address` is already used in `keys.rs`). If the inverse has a different name, use it and note it.

- [ ] **Step 3:** Create `src/cat.rs` with the puzzle-hash + balance reads (no spend yet) and a golden test:

```rust
//! DIG CAT coins over coinset: locate, value, and (later) spend the wallet's DIG.
use crate::coinset::ChainReads;
use crate::dig::DIG_ASSET_ID;
use crate::error::Result;
use chia::puzzles::cat::CatArgs;
use chia_protocol::Bytes32;
use chia_wallet_sdk::prelude::TreeHash;

/// The coinset puzzle hash where `owner_puzzle_hash`'s DIG CAT coins live.
pub fn dig_cat_puzzle_hash(owner_puzzle_hash: Bytes32) -> Bytes32 {
    let ph = CatArgs::curry_tree_hash(DIG_ASSET_ID, TreeHash::from(owner_puzzle_hash)).to_bytes();
    Bytes32::from(ph)
}

/// Total spendable DIG (base units) at the wallet's DIG CAT puzzle hash.
pub async fn dig_balance(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<u64> {
    let coins = chain.unspent_coins(dig_cat_puzzle_hash(owner_puzzle_hash)).await?;
    Ok(coins.iter().map(|c| c.amount).sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::keys::derive_wallet_keys;
    use chia_protocol::Coin;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn dig_cat_puzzle_hash_is_stable() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        // GOLDEN: capture the actual value on first run, then pin it here.
        let ph = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        assert_eq!(ph.to_bytes().len(), 32);
        // (Phase 1 Task 1.1 pins the exact hex once observed.)
    }

    #[tokio::test]
    async fn dig_balance_sums_cat_coins() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let cat_ph = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        mock.coins_by_ph.insert(cat_ph, vec![
            Coin::new(Bytes32::default(), cat_ph, 60_000),
            Coin::new(Bytes32::new([1u8;32]), cat_ph, 40_000),
        ]);
        assert_eq!(dig_balance(&mock, keys.owner_puzzle_hash).await.unwrap(), 100_000);
    }
}
```

  In `src/lib.rs` add `pub mod cat;`. Run `cargo test -p digstore-chain cat::` and `cargo test -p digstore-chain dig` — Expected PASS. Confirm `CatArgs::curry_tree_hash` + `TreeHash::from(Bytes32)` compile (the exact `TreeHash` import path may be `chia_wallet_sdk::prelude::TreeHash` or `chia::clvm_utils::TreeHash`; DataLayer-Driver `dig_coin.rs` uses `chia_wallet_sdk::prelude::TreeHash`).

- [ ] **Step 4:** Commit: `feat(chain): DIG constants + dig_cat_puzzle_hash + dig_balance`.

### Task 0.2: prototype — reconstruct DIG cats over coinset, send DIG+memo combined with a mint, push, confirm

**Files:** `crates/digstore-chain/examples/dig_payment_prototype.rs` (throwaway)

- [ ] **Step 1:** Write an async `#[tokio::main]` example that reads the test mnemonic from a path given by env `DIGSTORE_PROTO_CREDS` (do NOT hardcode; the runner points it at `.testcredentials`). It must:
  1. `derive_wallet_keys(phrase)`; `Coinset::mainnet()`.
  2. Print `dig_balance(owner_ph)`. **If 0, print "fund the wallet with DIG first" and exit** (no spend).
  3. Reconstruct the wallet's DIG CAT coins with lineage proofs over coinset: for each unspent coin at `dig_cat_puzzle_hash(owner_ph)`, fetch the parent coin record (`coin_record(parent_coin_info)`) + parent `coin_spend(parent_id, parent_spent_height)`, then `chia_wallet_sdk::driver::Cat::parse_children(&mut ctx, parent_coin, parent_puzzle, parent_solution)` and select the child matching `coin_id` with `info.asset_id == DIG_ASSET_ID` and a `lineage_proof`. **Record the exact calls + types** (this is the crux, mirroring `DigCoin::from_coin` but coinset-sourced).
  4. Build a CAT spend sending a SMALL test amount (e.g. `1000` base = 1 DIG) to `treasury_inner_puzzle_hash()`'s CAT, with `create_coin` memos `[treasury_inner_ph, fake_store_id]` (use a dummy 32-byte store id), change back to the owner DIG CAT ph. Use `chia_wallet_sdk::driver` CAT primitives (`Cat`/`CatSpend`/`Spends`/`Action`/`StandardLayer` — model on `DataLayer-Driver/src/dig_collateral_coin.rs`). **Record the working spend-construction calls.**
  5. Also build a real `mint_store` (empty root) using `singleton::build_mint`'s recipe; **concatenate** the mint `CoinSpend`s + the DIG CAT `CoinSpend`s; `datalayer_driver::sign_coin_spends(&all_spends, &[synthetic_sk], false)`; `SpendBundle::new(all_spends, sig)`; coinset `push`.
  6. Poll `coin_record(launcher_id)` + the treasury CAT coin to confirmed; print the launcher id, the treasury coin id, and confirm the memo is present.

- [ ] **Step 2:** Run manually on mainnet ONLY after the user confirms the wallet holds DIG and approves the spend: `DIGSTORE_PROTO_CREDS=.testcredentials cargo run -p digstore-chain --example dig_payment_prototype`. Iterate until the combined bundle confirms.

- [ ] **Step 3:** **Record findings** in this plan under "Phase 0-DIG findings": exact `Cat::parse_children` signature + how to source parent puzzle/solution over coinset; the exact CAT send+change spend-build calls (which `chia-wallet-sdk` `driver` types, how memos attach, how the inner standard spend is authorized); how the launcher id is obtained; how mint + CAT spends combine + sign together. Then `git rm` the example. Commit: `docs(plan): record DIG-CAT Phase 0 findings`.

> **Gate:** Phase 1+ CAT-spend code MUST be reconciled with these findings before implementation.

## Phase 0-DIG findings (record after Task 0.2)

_(to be filled in by Task 0.2 — exact CAT reconstruction + spend-build + combine/sign recipe)_

---

## Phase 1 — `cat.rs` production: `build_dig_payment` (per Phase 0 findings)

### Task 1.1: pin the golden CAT puzzle hash + finalize `dig_balance`
- [ ] Pin the observed `dig_cat_puzzle_hash(ABANDON owner)` hex into the Task 0.1 Step 3 golden test (replace the length-only assert with the exact value captured in Phase 0). Run `cargo test -p digstore-chain cat::`. Commit.

### Task 1.2: `build_dig_payment` + DIG cat reconstruction
**Files:** `crates/digstore-chain/src/cat.rs`
- [ ] Implement, per Phase 0 findings:
  - `pub async fn dig_cats(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<Vec<Cat>>` — reconstruct the wallet's DIG CAT coins with lineage proofs over coinset (the Phase-0 recipe).
  - `pub fn build_dig_payment(keys: &WalletKeys, dig_cats: &[Cat], amount: u64, store_id: Bytes32) -> Result<Vec<CoinSpend>>` — select cats covering `amount`; CAT-spend `amount` → treasury CAT ph with memos `[treasury_inner_ph, store_id]`; change → owner DIG CAT ph; authorize inner spends with `keys.synthetic_sk`'s standard layer; return the `CoinSpend`s. Error `ChainError::Chain("insufficient DIG: need N have M")` when cats don't cover `amount`.
- [ ] Test (offline, where mockable): insufficient-DIG selection → error; given canned `Cat`s (constructed in-test per the Phase-0 shapes), `build_dig_payment` emits a spend whose outputs include `amount` to the treasury CAT ph and a `store_id` memo (assert on the produced `CoinSpend` conditions). NOTE: full CAT lineage reconstruction (`dig_cats`) is validated live (Phase 0 + Phase 5), like `sync_datastore` — add an offline test only if a `Cat` can be constructed without real CLVM; otherwise gate `dig_cats` behind the live tests and unit-test only `build_dig_payment`'s selection/amount/memo logic. Commit.

---

## Phase 2 — embed the DIG payment in the anchor bundle

### Task 2.1: `anchor.rs` — combine DIG payment into mint/update + expose dig_balance
**Files:** `crates/digstore-chain/src/anchor.rs`
- [ ] Add to the `ChainAnchor` trait (and `CoinsetAnchor` impl): `async fn dig_balance(&self, keys: &WalletKeys) -> Result<u64>` = `crate::cat::dig_balance(&self.chain, keys.owner_puzzle_hash)`.
- [ ] In `mint_empty_store`: after `build_mint` (which yields `launcher_id` + the mint `SpendBundle`), reconstruct `dig_cats`, `build_dig_payment(keys, &cats, dig::INIT_DIG, launcher_id)`, **concatenate** the mint coin spends + DIG coin spends, re-sign the combined set with `synthetic_sk` (`sign_coin_spends(&all, &[sk], false)`), `SpendBundle::new(all, sig)`, single `push`. Return the same `MintOutcome`.
- [ ] In `update_root`: same, with `dig::COMMIT_DIG` and `store_id = launcher_id`.
- [ ] Test: extend the MockChain mint test to seed DIG cats at the owner's DIG CAT ph and assert `mint_empty_store` still pushes exactly ONE bundle (now containing mint + DIG spends). (Reconstruction/signature internals are live-validated.) `cargo test -p digstore-chain anchor::`. Commit.

> Reconcile the concatenation + single-signature step with Phase 0 findings (the synthetic key authorizes both the standard/XCH coins and the CAT inner spends).

---

## Phase 3 — CLI `balance` command + error variant

### Task 3.1: `CliError::InsufficientFunds { need, have, address, asset }`
**Files:** `crates/digstore-cli/src/error.rs`
- [ ] Change the variant to carry `asset: String` ("XCH" | "DIG"); update the `#[error(...)]` message to name the asset and the message/hint to print the correct receive address (XCH owner address for XCH; the same owner address for DIG — DIG is received as a CAT at the wallet). Update the existing constructor sites + the exit-code/hint tests. Keep exit code 12. Commit.

### Task 3.2: `digstore balance`
**Files:** `crates/digstore-cli/src/cli.rs`, `src/commands/mod.rs`, `src/commands/balance.rs` (new)
- [ ] `cli.rs`: add `Balance(BalanceArgs)` (no args beyond globals) + a clap parse test.
- [ ] `commands/balance.rs`: `run(ctx, ui)` → `ops::wallet::unlock_wallet_keys` → `anchor_backend::build_anchor()` → `block_on(anchor.balance(&keys))` (XCH) + `block_on(anchor.dig_balance(&keys))` (DIG). Human: print `XCH: <mojos> (<xch>)`, `DIG: <format_dig> (<base> base units)`, and the receive address (`digstore_chain::keys::owner_address`). `--json`: `{ xch_mojos, dig_base_units, dig, address, mocked }`. Mock seam: extend `MockAnchor` with a `dig_balance` returning a configurable value (env `DIGSTORE_ANCHOR_MOCK_DIG`, default large).
- [ ] Dispatch `Balance` (workspace-level — needs only the wallet, not a store; wire in the early block of `commands/mod.rs::dispatch` like `Seed`).
- [ ] Test (mock seam): `digstore balance` shows XCH + DIG (human + `--json`); `DIGSTORE_ANCHOR_MOCK_DIG=0` reflects zero DIG. Commit.

---

## Phase 4 — init/commit preflight gating

### Task 4.1: init preflight (XCH + DIG)
**Files:** `crates/digstore-cli/src/commands/init.rs`
- [ ] After unlocking keys + building the anchor, before the mint: `have_xch = block_on(anchor.balance(&keys))`; if `have_xch < fee + 1` → `InsufficientFunds{ asset:"XCH", need:fee+1, have:have_xch, address: owner_address }`. `have_dig = block_on(anchor.dig_balance(&keys))`; if `have_dig < dig::INIT_DIG` → `InsufficientFunds{ asset:"DIG", need:INIT_DIG, have:have_dig, address: owner_address }`. Both BEFORE any mint/scaffold.
- [ ] Test (mock seam): `DIGSTORE_ANCHOR_MOCK_DIG=0` → init fails exit 12 (asset DIG) and creates NO store dir; with DIG funded + XCH 0 → exit 12 (asset XCH). Commit.

### Task 4.2: commit preflight (XCH + DIG)
**Files:** `crates/digstore-cli/src/commands/commit.rs`
- [ ] After `stage_to_root` + unlocking keys, before `update_root`: require `have_xch >= fee` and `have_dig >= dig::COMMIT_DIG`; else `InsufficientFunds{asset}` (exit 12), no anchor, roots.log untouched.
- [ ] Test (mock seam): committed store, `DIGSTORE_ANCHOR_MOCK_DIG=0` → commit exit 12 (DIG), no new generation, staging intact. Commit.

---

## Phase 5 — live validation + docs

### Task 5.1: full workspace verification
- [ ] `cargo build --workspace`; `cargo test --workspace` (gated live tests ignored); `cargo clippy --workspace --all-targets`; `cargo fmt --check`. All green/clean. Commit any fmt.

### Task 5.2: live mainnet validation (gated; spends real DIG)
- [ ] Only after the user confirms the test wallet holds ≥ 110 DIG (`digstore balance`) and approves: real `digstore init` (pays 100 DIG, memo=launcher) → confirm the treasury received 100 DIG with the launcher memo on-chain; real `digstore commit` (pays 10 DIG) → confirm. Record the tx/coin ids. Do NOT run in CI.

### Task 5.3: docs
- [ ] README + `docs.dig.net` (command-reference + onchain-anchoring page): note the DIG cost (init 100 DIG, commit 10 DIG, paid to the DIG treasury in the same bundle), the `digstore balance` command, and that init/commit block on insufficient XCH/DIG. Commit. (Deploy to S3 + CloudFront invalidate as a separate step if requested.)

---

## Self-Review

**Spec coverage:** constants/amounts/memo (dig.rs, Task 0.1) ✔; CAT puzzle hash + balance (cat.rs, 0.1/1.1) ✔; CAT reconstruction + `build_dig_payment` (1.2, gated on Phase 0) ✔; embed in mint/update bundle atomically (2.1) ✔; `digstore balance` (3.2) ✔; XCH+DIG preflight blocking init (4.1) + commit (4.2) ✔; `InsufficientFunds{asset}` (3.1) ✔; Phase 0-DIG prototype-first (Phase 0) ✔; live validation (5.2) ✔; testing via MockChain + live (each phase) ✔; docs (5.3) ✔.

**Honesty about unknowns:** the exact `chia-wallet-sdk` CAT spend-construction + coinset lineage reconstruction is NOT fabricated here — it is pinned by the Phase 0-DIG prototype against the real compiler + mainnet, and Phase 1/2 are explicitly gated on those findings (mirroring the original anchoring plan's Phase 0). `dig_cat_puzzle_hash`, `dig_balance`, the preflight gating, the `balance` command, and the error variant are concrete real code testable offline now.

**Placeholder note:** the only intentionally-deferred internals are the CAT-spend calls behind the Phase 0 gate (a verification gate, not a hand-wave) and the golden CAT-ph hex (captured in Phase 0, pinned in 1.1). Everything else is complete.

**Type consistency:** `dig_cat_puzzle_hash(Bytes32)->Bytes32`, `dig_balance(&dyn ChainReads, Bytes32)->Result<u64>`, `build_dig_payment(&WalletKeys, &[Cat], u64, Bytes32)->Result<Vec<CoinSpend>>`, `ChainAnchor::dig_balance(&self,&WalletKeys)->Result<u64>`, `InsufficientFunds{need,have,address,asset}` — used consistently across tasks. Amounts `INIT_DIG=100_000`/`COMMIT_DIG=10_000` (base units) consistent.
