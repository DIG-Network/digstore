# HD-wallet support (digstore CLI) — design

**Date:** 2026-06-16
**Status:** Approved (pre-implementation)
**Repo:** `digstore_wasm` (the `digstore` CLI + `digstore-chain`)
**Scope:** Sub-project A of two. A = HD-wallet support (this doc) — fixes the wrong DIG/XCH balance + lets `digstore init`/mint actually spend funds spread across addresses. B = whole-CLI UX overhaul (separate spec/cycle).

## Problem

`digstore init` reported `1.000 DIG / 0.000000903383 XCH` and refused to mint, for a wallet that actually holds **8,390 DIG / 0.144 XCH**. Root cause: the CLI derives and checks **only address index 0**.

- `crates/digstore-chain/src/keys.rs:29` — `owner_puzzle_hash = master_public_key_to_first_puzzle_hash(master_pk)` (a single, index-0 address).
- `cat.rs::dig_balance` + `coinset.rs::unspent_coins` query that one puzzle hash.
- DIG asset id (`a406d3a9…`) is correct — not the cause.

Sage shows the **whole-wallet** aggregate; the coins sit at many HD-derived addresses the CLI never scans. Both XCH and DIG are wrong for the same reason. Spending (`dig_cats`, fee selection) has the same single-address blindness, so even with a correct balance, a mint couldn't gather the coins.

## Decisions (locked during brainstorming)

- **Full HD support:** aggregate balance across derived addresses AND select + sign spend coins across them (not consolidation-only).
- **Adaptive scan:** derive + batch-query addresses in chunks of 50; stop after a full chunk yields **zero** coins (XCH and DIG both empty); hard cap 500 addresses.
- **Unhardened derivation** (the standard Chia address path; observer-derivable), via `datalayer_driver`'s per-index wallet-key derivation.
- **TibetSwap link** in the insufficient-DIG error (`https://v2.tibetswap.io/`) — the one UX item folded in here; the rest is sub-project B.
- **Verification:** unit tests against the mock chain + one real `digstore init` against the test seed (`chia-scaled-parallel-voting/.test-credentials`). Multi-address signing correctness can only be fully trusted from a real run.

## Architecture

### 1. Key derivation — `crates/digstore-chain/src/keys.rs`
- New `IndexedKeys { index: u32, synthetic_sk, synthetic_pk, owner_puzzle_hash }`.
- New `derive_indexed_keys(mnemonic, indices: Range) -> Result<Vec<IndexedKeys>>` — for each unhardened index derive synthetic sk/pk + standard puzzle hash (the index-0 case must byte-match today's `derive_wallet_keys`).
- Keep `derive_wallet_keys` (index 0) — it's the receive address shown to users / used for the "fund this" address and the treasury memo owner.
- Confirm the datalayer_driver fn for per-index unhardened wallet keys (e.g. `master_sk_to_wallet_sk_unhardened(master, i)` / equivalent); if only index-0 helpers are re-exported, derive via the underlying chia derivation the SDK wraps.

### 2. Adaptive wallet scan — new `crates/digstore-chain/src/wallet.rs`
- `struct ScannedWallet { keys: Vec<IndexedKeys> }` built by `scan_wallet(chain, mnemonic) -> Result<ScannedWallet>`:
  - loop chunks of 50 indices; for each chunk, batch `get_coin_records_by_puzzle_hashes` over both the XCH phs and the DIG-CAT phs; keep indices that have any coins (always keep index 0); stop when a full chunk is entirely empty; cap 500.
  - Records, per kept index, its XCH coins and DIG coins (so balance + selection reuse the scan, no re-query).
- `xch_balance()` / `dig_balance()` sum across kept indices.
- This is the single entry point the anchor/init flow uses; balance display and spend selection both read from it.

### 3. Balance — `cat.rs` / `anchor.rs`
- `dig_balance`/`balance` become "sum over the scanned wallet" (delegate to `ScannedWallet`). Keep the single-ph helpers (`dig_cat_puzzle_hash`, `unspent_coins`) as building blocks.

### 4. Spend / mint — `cat.rs::dig_cats`, `build_dig_payment`, `anchor.rs`
- `dig_cats` gathers DIG cats across all scanned indices, each tagged with its owning index.
- Greedy-select (largest-first) to cover the amount; the result carries, per selected coin, the `synthetic_sk` needed to sign it.
- `build_dig_payment` + the XCH-fee path build spends from the multi-address selection; change returns to index 0 (consolidating over time).
- Signing: aggregate-sign the bundle with the set of distinct synthetic keys the selected coins require (not just index 0's).

### 5. Insufficient-DIG error — `cat.rs` / `error.rs`
On aggregate shortfall: message shows aggregate DIG held, the index-0 receive address to fund, and `Acquire DIG on TibetSwap: https://v2.tibetswap.io/`. (Color/formatting polish is sub-project B; the link + accurate aggregate land here.)

## Error handling
- Coinset query failure during scan → propagate `ChainError::Chain` (don't silently undercount).
- Cap reached (500) without an empty chunk → proceed with what's found + a warning (a wallet spread beyond 500 addresses is pathological).
- Mnemonic invalid → existing `InvalidMnemonic`.

## Testing
- **keys:** `derive_indexed_keys` index-0 matches `derive_wallet_keys`; indices are distinct + stable.
- **scan/balance:** mock `ChainReads` seeded with DIG/XCH coins at several indices → `xch_balance`/`dig_balance` equal the cross-address sum; adaptive scan stops on the empty chunk; index-0-only wallet still works.
- **selection:** multi-address greedy selection covers the amount + reports the right per-coin keys; shortfall returns the insufficient-DIG error (with the TibetSwap link).
- **manual (required):** `digstore init` against the test seed (`chia-scaled-parallel-voting/.test-credentials`) — confirm the reported balance matches Sage and a real mint signs + broadcasts. If that seed's funds are NOT spread, signing across addresses stays partly unverified (note it).

## Out of scope (→ sub-project B)
- Color/spinner/progress-bar output, redundancy cleanup, restyling other commands.
- Auto-consolidation as a standalone command.
