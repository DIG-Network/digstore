# DIG CAT Payment + Balance Gating — Design

**Date:** 2026-06-11
**Status:** Approved (design)
**Scope:** Every on-chain store operation pays the DIG treasury in the DIG CAT, embedded in the same spend bundle as the singleton mint/update. Add XCH+DIG balance reporting and gate store operations on sufficient funds.

## Summary

Anchoring a digstore store on Chia already mints a singleton (`init`) and updates its root (`commit`). This feature adds a **DIG CAT payment** to each of those spend bundles:

- **`init`** (mint) → also sends **100 DIG** to the DIG treasury address, with a memo carrying the new store id (the launcher id).
- **`commit`** (root update) → also sends **10 DIG** to the same treasury address, memo = the store id.

The DIG payment rides in the **same signed spend bundle** as the singleton spend (atomic: both confirm or neither). A new `digstore balance` reports spendable XCH and DIG; `init`/`commit` **preflight** the wallet and **block before any spend** if XCH or DIG is short.

## Constants (locked)

| Thing | Value |
|---|---|
| DIG CAT asset id | `a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81` (matches DataLayer-Driver `DIG_ASSET_ID`) |
| Treasury recipient | `xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y` (bech32 → recipient inner puzzle hash) |
| DIG decimals | 3 (1 DIG = 1000 base units) |
| init payment | 100 DIG = 100_000 base units |
| commit payment | 10 DIG = 10_000 base units |
| memo (init AND commit) | the store id = launcher id (raw 32 bytes), as an extra memo beyond the recipient hint |
| extra XCH for the DIG payment | none — the bundle's existing XCH `fee` is unchanged; the DIG payment moves CAT value only |

## Background (what exists)

- `digstore-chain` builds singleton spends at the `datalayer-driver` / `chia-wallet-sdk` level and broadcasts via **coinset** (`chia-sdk-coinset`), no P2P peer. `singleton.rs` has `build_mint`, `build_update`, `sync_datastore` (lineage walk over coinset); `anchor.rs` has `CoinsetAnchor` with `mint_empty_store`/`update_root`/`confirm`/`balance` (XCH).
- `coinset.rs` `ChainReads`: `unspent_coins(ph)`, `coin_record(name)`, `coin_spend(coin_id, height)` (= `get_puzzle_and_solution`), `peak_height`, `push`. `classify_coin_record` treats coinset "not found" as pending.
- DataLayer-Driver (reference, P2P-based — NOT used directly): `DIG_ASSET_ID`; `DigCoin::puzzle_hash(owner_ph) = CatArgs::curry_tree_hash(DIG_ASSET_ID, owner_ph)`; `DigCoin::from_coin` validates a CAT via `Cat::parse_children` over a Peer. We reproduce the CAT-coin reconstruction over **coinset** instead (same primitives the singleton sync uses).
- Owner keys: `keys.rs` `WalletKeys { synthetic_sk, synthetic_pk, owner_puzzle_hash }`. The owner's **standard** puzzle (synthetic key) is the CAT **inner** puzzle, so the same synthetic key signs both the singleton/XCH spends and the DIG CAT spends — one signature set over the combined bundle.

## Architecture

### New module `digstore-chain/src/cat.rs` — DIG CAT over coinset

One responsibility: find, value, and spend DIG CAT coins for the wallet.

- `pub fn dig_cat_puzzle_hash(owner_puzzle_hash: Bytes32) -> Bytes32` = `CatArgs::curry_tree_hash(DIG_ASSET_ID, owner_puzzle_hash.into())`. The coinset puzzle-hash where the wallet's DIG coins live.
- `pub async fn dig_balance(chain: &dyn ChainReads, owner_puzzle_hash: Bytes32) -> Result<u64>` — sum the `amount` of unspent coins at `dig_cat_puzzle_hash(owner_ph)` (base units). (Add `ChainReads` already exposes `unspent_coins`; balance reuses it against the CAT ph.)
- DIG CAT-coin reconstruction over coinset: for each unspent DIG coin, fetch its parent coin record + parent `get_puzzle_and_solution`, run `chia_wallet_sdk::driver::Cat::parse_children(parent_coin, parent_puzzle, parent_solution)` and select the child whose `coin_id` matches and `info.asset_id == DIG_ASSET_ID`, capturing its `lineage_proof`. (Mirror `DigCoin::from_coin`, but read parent state via coinset, not a Peer.)
- `pub fn build_dig_payment(keys: &WalletKeys, dig_cats: &[Cat], amount: u64, store_id: Bytes32) -> Result<Vec<CoinSpend>>`:
  1. select DIG cats covering `amount`;
  2. build a CAT spend sending `amount` to the **treasury CAT puzzle hash** (`CatArgs::curry_tree_hash(DIG_ASSET_ID, TREASURY_INNER_PH)`), with `create_coin` memos `[TREASURY_INNER_PH (hint), store_id]`;
  3. change (selected − amount) back to the owner's DIG CAT ph (hint = owner inner ph);
  4. authorize the inner spends with the synthetic key (standard layer). No XCH reserve (CAT-value only).
  Built with `chia_wallet_sdk::driver` CAT primitives (`Cat`, `CatSpend`/`Spends`/`Action`, `StandardLayer`), returning the CAT `CoinSpend`s.

### `anchor.rs` — embed the payment in the bundle

- `mint_empty_store(keys, fee)`: build the mint (gives launcher id), then `build_dig_payment(keys, dig_cats, 100_000, launcher_id)`, **concatenate** the mint coin spends + DIG coin spends into one `SpendBundle`, `sign_coin_spends` over the combined set with `synthetic_sk`, single `push`. Atomic.
- `update_root(launcher_id, new_root, keys, fee)`: same, with `10_000` DIG and `store_id = launcher_id`.
- The DIG cats are fetched inside the anchor (reconstructed over `self.chain`). Add an internal helper `dig_cats_for(keys)` and `dig_balance` to the anchor/`ChainReads` surface as needed.

### CLI

- New `commands/balance.rs` + `Command::Balance`: `digstore balance` prints spendable **XCH** (mojos + XCH) and **DIG** (base units ÷ 1000) for the unlocked wallet, plus the receive address (`puzzle_hash_to_address(owner_ph,"xch")` and the DIG note); `--json` structured. Read-only — unlocks the seed to derive keys, no spend. Store-independent (workspace-level; needs only the wallet).
- `init` preflight (in `commands/init.rs`): after unlocking keys, require **XCH ≥ fee+1** and **DIG ≥ 100_000**; else `InsufficientFunds` naming the short asset + receive address, **before** the mint. `commit` preflight (`commands/commit.rs`, after `stage_to_root`): require **XCH ≥ fee** and **DIG ≥ 10_000**.
- Error: extend `CliError::InsufficientFunds` to `{ need, have, address, asset }` (asset = "XCH" | "DIG") so the message and hint name the right token + address; map from a chain-layer `ChainError` shortfall.

## Data flow

```
init:   unlock keys → balance preflight (XCH≥fee+1, DIG≥100k) →
        build_mint → launcher_id → build_dig_payment(100k, memo=launcher) →
        bundle = mint_spends ++ dig_spends → sign(synthetic_sk) → coinset push → confirm
commit: stage_to_root → unlock keys → preflight (XCH≥fee, DIG≥10k) →
        sync_datastore → build_update → build_dig_payment(10k, memo=store_id) →
        bundle = update_spends ++ dig_spends → sign → push → confirm → finalize
balance: unlock keys → XCH = Σ unspent@owner_ph ; DIG = Σ unspent@dig_cat_ph(owner_ph)
```

## Error handling

- Insufficient XCH or DIG → blocked at preflight (`InsufficientFunds{asset}`), no spend, no local state.
- DIG coin reconstruction / selection failure → `MintFailed`/`UpdateFailed`; nothing persisted (mint half: covered by the existing post-mint scaffold rules — but the DIG payment is in the SAME bundle, so a bundle that fails to build/sign/push fails the whole op before any local scaffold for init; for commit, finalize only runs after confirm).
- Coinset "not found" while reconstructing CAT parents → treat like the singleton path (pending/absent vs hard error), surfaced clearly.

## Phase 0-DIG prototype (throwaway, FIRST)

Before production code, a gated example proves the path on mainnet:
1. `dig_balance` of the test wallet (read-only) — confirm it holds DIG; if zero, STOP and report (fund the wallet first).
2. Reconstruct the wallet's DIG CAT coins + lineage over coinset.
3. Build a DIG CAT payment (a small amount) to the treasury with a memo; combine with a real `mint_store` spend into one bundle; `sign`; coinset `push`; poll to confirmed; verify the treasury received the CAT with the memo.
Record the working CAT-reconstruction + spend-build recipe, then delete the prototype (like anchoring Phase 0). Production `cat.rs` is written against the proven recipe.

## Testing

- **MockChain**: add DIG CAT coin records at `dig_cat_puzzle_hash(owner_ph)` + canned parent spends so `Cat::parse_children` reconstructs lineage offline.
- **Unit**: `dig_cat_puzzle_hash` golden; `dig_balance` sums CAT coins; `build_dig_payment` produces a spend sending the exact `amount` to the treasury CAT ph with memos `[hint, store_id]` and correct change; insufficient-DIG selection errors.
- **CLI**: `digstore balance` (human + json, XCH+DIG) via the mock seam; init/commit preflight **blocks** on insufficient DIG (mock `dig_balance` = 0 → `InsufficientFunds{asset:"DIG"}`, no store/generation).
- **Live**: the Phase 0-DIG prototype, then one real `init` (100 DIG) + one real `commit` (10 DIG) on the funded test wallet, verifying the treasury coin + memo on-chain.

## Out of scope

- Reclaiming/refunding DIG; treasury-side accounting; DIG on `anchor` resume (resume only confirms an already-submitted bundle).
- Configurable amounts/recipient (constants for now).
- Non-mainnet.

## Security notes

- The DIG payment is authorized by the same synthetic key as the singleton spend; one signature set over the combined bundle. No new key material.
- Amounts/recipient/asset id are compiled-in constants (not attacker-influenced).
- Preflight is a UX guard; the chain still enforces the spend (a bypassed preflight just yields a failed bundle, not a bad local state).
