# HD-Wallet Support (digstore CLI) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make `digstore` see + spend the whole HD wallet (all derived addresses), fixing the wrong balance and the `init`/mint failure when funds are spread across addresses.

**Architecture:** Add adaptive HD address derivation + a wallet scan in `digstore-chain`; aggregate XCH/DIG balance across all scanned addresses; gather + select spend coins across addresses and sign the bundle with all involved synthetic keys (`sign_coin_spends` already accepts a slice). Add the TibetSwap refill link to the insufficient-DIG error.

**Tech Stack:** Rust, `datalayer_driver` (key derivation, coin spends, signing), coinset reads, the `digstore-chain` crate consumed by `digstore-cli`.

**Reference spec:** `docs/superpowers/specs/2026-06-16-hd-wallet-support-design.md`

## Key facts (verified)
- `keys.rs` derives via `datalayer_driver`: `master_public_key_to_first_puzzle_hash`, `master_secret_key_to_wallet_synthetic_secret_key`. Index 0 only today.
- `cat.rs::dig_balance(chain, owner_ph)` + `coinset.rs::unspent_coins(ph)` query ONE ph. `dig_cat_puzzle_hash(owner_ph) = CatArgs::curry_tree_hash(DIG_ASSET_ID, owner_ph)`. DIG_ASSET_ID `a406d3a9…` is correct.
- `anchor.rs::mint_empty_store(keys, fee)`: XCH from `unspent_coins(keys.owner_puzzle_hash)` → `build_mint_unsigned(keys, &unspent, Bytes32::default(), fee)` (singleton.rs:151); DIG from `dig_cats(...)` → `build_dig_payment(keys, &cats, dig::INIT_DIG, launcher_id)` (cat.rs:148); combine; `sign_coin_spends(&all, std::slice::from_ref(&keys.synthetic_sk), false)`.
- **`sign_coin_spends(coin_spends, secret_keys: &[SecretKey], false)` takes a SLICE** — passing all kept indices' synthetic keys signs every coin correctly. This is the crux that makes multi-address signing simple.
- `init.rs` calls `anchor.balance(&keys)` + `anchor.dig_balance(&keys)` for the preflight display, then `mint_empty_store(&keys, fee)`.

## File structure
- `crates/digstore-chain/src/keys.rs` — add `IndexedKeys` + `derive_indexed_keys`.
- `crates/digstore-chain/src/wallet.rs` — NEW: adaptive scan + aggregate balances + per-coin key map.
- `crates/digstore-chain/src/lib.rs` — `pub mod wallet;`
- `crates/digstore-chain/src/cat.rs` — multi-address DIG balance + `dig_cats_multi` + insufficient-DIG error w/ TibetSwap.
- `crates/digstore-chain/src/anchor.rs` — `balance`/`dig_balance` aggregate; `mint_empty_store`/`update_root` gather across addresses + sign with all keys.
- `crates/digstore-chain/src/singleton.rs` — `build_mint_unsigned` accept a multi-address XCH coin set + explicit change ph.
- `crates/digstore-cli/src/commands/init.rs` — display aggregate balance; pass the scanned wallet.

---

## Task 1: Indexed key derivation

**Files:** `crates/digstore-chain/src/keys.rs`

- [ ] **Step 1: Failing test** — add to the `tests` mod:
```rust
    #[test]
    fn indexed_keys_index0_matches_single() {
        let single = derive_wallet_keys(ABANDON).unwrap();
        let many = derive_indexed_keys(ABANDON, 0..3).unwrap();
        assert_eq!(many.len(), 3);
        assert_eq!(many[0].index, 0);
        assert_eq!(many[0].owner_puzzle_hash, single.owner_puzzle_hash);
        // distinct addresses per index
        assert_ne!(many[0].owner_puzzle_hash, many[1].owner_puzzle_hash);
        assert_ne!(many[1].owner_puzzle_hash, many[2].owner_puzzle_hash);
    }
```
- [ ] **Step 2: Run** `cargo test -p digstore-chain indexed_keys_index0_matches_single` → FAIL (no `derive_indexed_keys`).
- [ ] **Step 3: Implement** in `keys.rs`:
```rust
/// One wallet address (unhardened index) + its signing key.
#[derive(Clone)]
pub struct IndexedKeys {
    pub index: u32,
    pub synthetic_sk: SecretKey,
    pub synthetic_pk: PublicKey,
    pub owner_puzzle_hash: Bytes32,
}

/// Derive the wallet keys for a range of unhardened indices. Index 0 byte-matches
/// `derive_wallet_keys` (the legacy single-address path).
pub fn derive_indexed_keys(
    mnemonic: &str,
    indices: std::ops::Range<u32>,
) -> Result<Vec<IndexedKeys>> {
    let m = Mnemonic::parse_normalized(mnemonic.trim())
        .map_err(|e| ChainError::InvalidMnemonic(e.to_string()))?;
    let seed = Zeroizing::new(m.to_seed(""));
    let master_sk = SecretKey::from_seed(seed.as_ref());
    let mut out = Vec::new();
    for index in indices {
        let synthetic_sk = wallet_synthetic_sk_for_index(&master_sk, index);
        let synthetic_pk = secret_key_to_public_key(&synthetic_sk);
        let owner_puzzle_hash = synthetic_pk_to_owner_ph(&synthetic_pk);
        out.push(IndexedKeys { index, synthetic_sk, synthetic_pk, owner_puzzle_hash });
    }
    Ok(out)
}
```
> NOTE — derivation helpers: index 0 must reproduce `master_public_key_to_first_puzzle_hash(master_pk)` + `master_secret_key_to_wallet_synthetic_secret_key(master_sk)`. INVESTIGATE `datalayer_driver`'s exported derivation fns for the per-index unhardened wallet key (grep the crate for `wallet_sk`, `unhardened`, `to_wallet_synthetic`, `master_sk_to_wallet_sk`). Implement `wallet_synthetic_sk_for_index(master_sk, i)` and `synthetic_pk_to_owner_ph(pk)` using them so index 0 matches the legacy result. If datalayer_driver only re-exports the index-0 helpers, use the chia derivation crate it depends on (`chia_bls` / `chia-wallet`) directly for the unhardened child path `m/12381'/8444'/2/i` then the synthetic-offset + standard-puzzle-hash. The index-0-match test is the oracle — make it pass.

- [ ] **Step 4: Run** the test → PASS.
- [ ] **Step 5: Commit** `git add crates/digstore-chain/src/keys.rs && git commit -m "feat(wallet): derive_indexed_keys (unhardened HD range; index 0 matches legacy)"`

---

## Task 2: Adaptive wallet scan + aggregate balances

**Files:** Create `crates/digstore-chain/src/wallet.rs`; modify `crates/digstore-chain/src/lib.rs`

- [ ] **Step 1: Failing test** (new `wallet.rs` `tests` mod, using the existing mock `ChainReads` pattern from `cat.rs`/`anchor.rs` tests — copy that mock):
```rust
    #[tokio::test]
    async fn scan_aggregates_xch_and_dig_across_indices() {
        // mock chain: XCH coins at index 0 + index 2 phs; DIG cats at index 1 ph.
        // (Derive the phs with derive_indexed_keys(ABANDON, 0..3).)
        // Seed amounts so totals are unambiguous, then assert xch_balance + dig_balance.
        let w = scan_wallet(&mock, ABANDON).await.unwrap();
        assert_eq!(w.xch_balance(), /* sum of seeded XCH */);
        assert_eq!(w.dig_balance(), /* sum of seeded DIG */);
    }
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `wallet.rs`:
```rust
//! Adaptive HD wallet scan: derive addresses in chunks and aggregate the wallet's
//! XCH + DIG coins across all of them (Sage-style whole-wallet balance), so the CLI
//! no longer sees only index 0.
use crate::cat::dig_cat_puzzle_hash;
use crate::coinset::ChainReads;
use crate::error::Result;
use crate::keys::{derive_indexed_keys, IndexedKeys};
use chia_protocol::{Bytes32, Coin};

const CHUNK: u32 = 50;
const MAX_INDEX: u32 = 500;

/// Per-address coins discovered by the scan.
pub struct AddressCoins {
    pub keys: IndexedKeys,
    pub xch: Vec<Coin>,
    pub dig: Vec<Coin>, // raw DIG CAT coins at this address's DIG ph
}

pub struct ScannedWallet {
    pub addrs: Vec<AddressCoins>,
}

impl ScannedWallet {
    pub fn xch_balance(&self) -> u64 {
        self.addrs.iter().flat_map(|a| &a.xch).map(|c| c.amount).sum()
    }
    pub fn dig_balance(&self) -> u64 {
        self.addrs.iter().flat_map(|a| &a.dig).map(|c| c.amount).sum()
    }
    /// All synthetic secret keys for addresses that hold any coins (for signing).
    pub fn signing_keys(&self) -> Vec<chia_bls::SecretKey> {
        self.addrs.iter().map(|a| a.keys.synthetic_sk.clone()).collect()
    }
}

/// Scan in chunks of CHUNK indices; stop after a full chunk with NO coins; cap MAX_INDEX.
pub async fn scan_wallet(chain: &dyn ChainReads, mnemonic: &str) -> Result<ScannedWallet> {
    let mut addrs = Vec::new();
    let mut start = 0u32;
    while start < MAX_INDEX {
        let keys = derive_indexed_keys(mnemonic, start..(start + CHUNK))?;
        let mut chunk_has_any = false;
        for k in keys {
            let xch = chain.unspent_coins(k.owner_puzzle_hash).await?;
            let dig = chain.unspent_coins(dig_cat_puzzle_hash(k.owner_puzzle_hash)).await?;
            if !xch.is_empty() || !dig.is_empty() || k.index == 0 {
                if !xch.is_empty() || !dig.is_empty() { chunk_has_any = true; }
                addrs.push(AddressCoins { keys: k, xch, dig });
            }
        }
        if !chunk_has_any { break; }
        start += CHUNK;
    }
    Ok(ScannedWallet { addrs })
}
```
> NOTE: `signing_keys`/`SecretKey` type — use the same `SecretKey` type `keys.rs` uses (re-exported from `datalayer_driver`); fix the import. The per-coin batching here is one query per (ph, asset); if `ChainReads` exposes a batch `get_coin_records_by_puzzle_hashes` wrapper, prefer batching a whole chunk's phs in one call for speed — but the simple per-ph loop is correct and fine to ship first.

- [ ] **Step 4: Add** `pub mod wallet;` to `lib.rs`. **Run** the test → PASS.
- [ ] **Step 5: Commit** `git add crates/digstore-chain/src/wallet.rs crates/digstore-chain/src/lib.rs && git commit -m "feat(wallet): adaptive HD scan + aggregate XCH/DIG balance"`

---

## Task 3: Aggregate balance in the anchor + TibetSwap error

**Files:** `crates/digstore-chain/src/anchor.rs`, `crates/digstore-chain/src/cat.rs`

- [ ] **Step 1: Failing test** — in `anchor.rs` tests, seed the mock with DIG/XCH at indices 0 and 2, build the anchor, and assert `balance`/`dig_balance` return the cross-address sum (mirror the existing `balance_sums_coins_at_owner_ph` test but with a multi-index mock). Also add (cat.rs) a test that the insufficient-DIG error string contains `tibetswap`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
  - The `ChainAnchor` needs the mnemonic to scan (today it only gets `keys`). Add a scan: change `balance`/`dig_balance` to take the mnemonic (or a `&ScannedWallet`). Cleanest minimal: add `async fn scan(&self, mnemonic: &str) -> Result<ScannedWallet>` to the anchor and have `balance`/`dig_balance` accept `&ScannedWallet` (so the CLI scans once and reuses). Update the trait + `init.rs` callers accordingly.
    ```rust
    async fn balance(&self, w: &ScannedWallet) -> Result<u64> { Ok(w.xch_balance()) }
    async fn dig_balance(&self, w: &ScannedWallet) -> Result<u64> { Ok(w.dig_balance()) }
    async fn scan(&self, mnemonic: &str) -> Result<ScannedWallet> {
        crate::wallet::scan_wallet(&self.chain as &dyn ChainReads, mnemonic).await
    }
    ```
  - In `cat.rs` `select_dig_cats` shortfall error, append the TibetSwap link:
    ```rust
    return Err(ChainError::Chain(format!(
        "insufficient DIG: need {amount} have {sum}. Acquire DIG on TibetSwap: https://v2.tibetswap.io/"
    )));
    ```
> NOTE: changing the `Anchor` trait signatures touches `init.rs` + any other caller + the mock impls in tests. Update them all. Keep `dig_cat_puzzle_hash`/single-ph `dig_balance` as building blocks (the scan uses them).

- [ ] **Step 4: Run** `cargo test -p digstore-chain` → PASS.
- [ ] **Step 5: Commit** `git add -A crates/digstore-chain && git commit -m "feat(wallet): aggregate anchor balance over scanned wallet + TibetSwap link on shortfall"`

---

## Task 4: Multi-address mint (spend across addresses, sign with all keys)

**Files:** `crates/digstore-chain/src/anchor.rs`, `crates/digstore-chain/src/cat.rs`, `crates/digstore-chain/src/singleton.rs`

- [ ] **Step 1: Implement multi-address coin gathering + signing in `mint_empty_store`.**
  Rework it to take the scanned wallet and spend across addresses:
  - XCH for the mint coin/fee: flatten `w.addrs[*].xch` (tag each coin with its `owner_puzzle_hash`/`synthetic_pk` so the spend can be built); change returns to index 0 (`w.addrs[0].keys`).
  - DIG: gather DIG cats across all addresses (Task: generalize `dig_cats` → `dig_cats_for(chain, owner_ph)` per address, call for each scanned DIG-bearing address, concat) then `select_dig_cats` over the combined set.
  - Sign with ALL involved keys: `sign_coin_spends(&all, &w.signing_keys(), false)` (slice of every scanned address's synthetic_sk).
  ```rust
  let signature = sign_coin_spends(&all, &w.signing_keys(), false)
      .map_err(|e| ChainError::Chain(format!("sign combined mint+DIG bundle: {e}")))?;
  ```
- [ ] **Step 2: Adapt `build_mint_unsigned` (singleton.rs:151) + `build_dig_payment` (cat.rs:148)** to accept a multi-address coin set and an explicit change/owner puzzle hash (index 0), instead of a single `keys`.
> NOTE — this is the deep part. READ `build_mint_unsigned` + `build_dig_payment` fully first. They currently assume one `keys` (one synthetic_pk for the spend + one change ph). Generalize so each input coin is spent under ITS address's standard puzzle (synthetic_pk), with change consolidated to index 0. The signing is already handled (all keys passed to `sign_coin_spends`), so the builders only need to emit each input coin's spend with the correct per-coin puzzle reveal. If `datalayer_driver`'s spend helpers bake in a single key, build the per-coin standard spends explicitly using each coin's `synthetic_pk`. Keep the bundle atomic (one SpendBundle, one aggregate signature). Preserve the DIG memo = launcher_id and the treasury destination.
- [ ] **Step 3: Tests** — extend the mock-chain tests so a mint draws DIG from TWO addresses and XCH from another; assert the built bundle includes coin spends for coins at >1 distinct puzzle hash, and that `signing_keys()` covers them. (Bundle validity itself is verified live in Task 5.)
- [ ] **Step 4: Run** `cargo test -p digstore-chain` → PASS.
- [ ] **Step 5: Commit** `git add -A crates/digstore-chain && git commit -m "feat(wallet): mint spends DIG+XCH across HD addresses, signs with all keys"`

---

## Task 5: CLI wiring + display + live verification

**Files:** `crates/digstore-cli/src/commands/init.rs` (+ any other command calling balance/mint)

- [ ] **Step 1: Update `init.rs`** to scan once and reuse:
  - `let w = block_on(anchor.scan(&mnemonic))??;`
  - `let have_xch = w.xch_balance();  let have_dig = w.dig_balance();`
  - pass `&w` to `mint_empty_store`.
  - The "fund this" address stays index 0 (`owner_address(&derive_wallet_keys(&mnemonic)?)`).
- [ ] **Step 2: Update other callers** — grep `dig_balance|\.balance(|mint_empty_store|update_root` across `crates/digstore-cli/src` and fix each to the new signatures (e.g. `balance.rs`, `commit.rs`). Build the whole workspace.
- [ ] **Step 3: Build + unit tests** — `cargo build -p digstore-cli && cargo test -p digstore-chain -p digstore-cli` → PASS.
- [ ] **Step 4: LIVE verification (required).** Run a real `digstore init` against the test seed at `chia-scaled-parallel-voting/.test-credentials`:
  - confirm the reported XCH + DIG balance matches the wallet's true aggregate (cross-check on spacescan/Sage);
  - if the wallet holds ≥100 DIG + fee, confirm the mint signs + broadcasts (tx id returned, store created);
  - if that seed's funds are NOT spread across addresses, note that multi-address signing remains verified only structurally (Task 4 Step 3), and flag for a real-wallet run.
  Document the command + observed output.
- [ ] **Step 5: Commit** `git add -A && git commit -m "feat(wallet): init scans the whole HD wallet for balance + mint"`

---

## Self-Review notes
- **Spec coverage:** range derivation → T1; adaptive scan + aggregate → T2; anchor balance + TibetSwap → T3; cross-address spend + multi-key signing → T4; CLI display + live verify → T5. Covered.
- **Type consistency:** `IndexedKeys`/`derive_indexed_keys` (T1) ↔ `wallet.rs` (T2) ↔ anchor/init (T3,T5); `ScannedWallet::{xch_balance,dig_balance,signing_keys}` ↔ T3/T4/T5; `sign_coin_spends(&all, &w.signing_keys(), false)` consistent.
- **Flagged investigations (NOTE):** datalayer_driver per-index derivation fn (T1 — index-0-match test is the oracle); batch coinset query (T2 perf, optional); trait-signature ripple to callers/mocks (T3,T5); the `build_mint_unsigned`/`build_dig_payment` multi-address rework (T4 — the deep, live-verified part).
- **Risk:** T4 multi-address spend-building is the one part unit tests can't fully prove; T5 Step 4 live run against the test seed is the gate. If the seed lacks spread funds, signing across addresses needs a real-wallet `digstore init` to finalize.
