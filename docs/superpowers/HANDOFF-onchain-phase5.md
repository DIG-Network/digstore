# Handoff — Onchain Anchoring, Phase 5 (CLI integration)

Paste the prompt below into a fresh Claude Code session in `C:\Users\micha\workspace\dig_network\digstore_wasm` to resume.

---

Resume the digstore onchain-anchoring feature — Phase 5 (CLI integration), the final stretch. The hard half is already done, reviewed, and proven on Chia mainnet.

**First, read these (in order):**
1. Memory: `onchain-anchoring-progress.md` (auto-loaded) — full status, every commit, the remaining flow.
2. Plan: `docs/superpowers/plans/2026-06-11-onchain-anchoring.md` — esp. the "Phase 0 findings" section (pinned API/recipe) and Phase 5 tasks.
3. Spec: `docs/superpowers/specs/2026-06-11-onchain-anchoring-design.md` (esp. "Verification spike results").

**State:**
- Branch: `feature/onchain-anchoring` (do NOT switch). Plan 1 (seed mgmt) already merged to `main`.
- `digstore-chain` crate is COMPLETE + reviewed: `coinset.rs` (`ChainReads`/`Coinset::mainnet()`/`MockChain`), `keys.rs` (`derive_wallet_keys -> WalletKeys{synthetic_sk,synthetic_pk,owner_puzzle_hash}`), `singleton.rs` (`build_mint`, `sync_datastore`, `build_update`), `anchor.rs` (`ChainAnchor` trait + `CoinsetAnchor<C>::mainnet()`; `MintOutcome{launcher_id,coin_id}`, `UpdateOutcome{new_coin_id}`, `ConfirmState`). All proven; a real store `cf915cbaac0755db8c79b1b2e3b2eadf14d14f7246bb7e05d951802cd273211c` was minted on mainnet.
- `digstore-cli` Phase 5.1 done: error variants `InsufficientFunds{need,have,address}`/`Chain`/`ConfirmTimeout`/`MintFailed`/`UpdateFailed` + hints; `src/runtime.rs::block_on` drives async `ChainAnchor` from the sync command dispatch.

**Method:** subagent-driven-development (superpowers) — implementer + spec review + code-quality review per task, like the prior phases. Commits: NO Co-Authored-By trailer; SSH-signed; conventional messages.

**Build note:** `digstore-cli` has a build-script (contract D6) needing the guest wasm. If a build panics about missing `digstore_guest.wasm`, run `cargo build -p digstore-guest --target wasm32-unknown-unknown --release` first.

**Test wallet:** `.testcredentials` at repo root (gitignored, 24-word mainnet mnemonic, ~904k mojos, address `xch1htza92lz...`). Authorized to spend real XCH. Read it at runtime; never echo/commit it.

**⚠ This phase spends real mainnet XCH** (init mints; every commit updates onchain) and rewrites existing user-facing `init`/`commit`. Work carefully; confirm before money-spending live runs.

**Remaining tasks (Phase 5):**

- **5.2 — `init` mints (hard gate).** Read `crates/digstore-cli/src/commands/init.rs` + `ops/store_ops.rs::init_store` first. Change: **the store_id becomes the onchain launcher id** from `mint_empty_store` (drop the old `SHA256(pubkey)` derivation). Flow: resolve+unlock seed (reuse the Plan-1 seed-mgmt `resolve_passphrase` in `commands/seed.rs` + `digstore_chain::{config,seed,unlock}`) → `digstore_chain::derive_wallet_keys(phrase)` → `CoinsetAnchor::mainnet()` → preflight `anchor.balance(&keys)`; if short → `CliError::InsufficientFunds` with receive address (`datalayer_driver::puzzle_hash_to_address(keys.owner_puzzle_hash,"xch")`) → `runtime::block_on(anchor.mint_empty_store(&keys, fee))` → write the local store keyed on `launcher_id` + an `[anchor]` table (status=pending) → `anchor.confirm(coin_id, timeout)` → flip to confirmed. Pre-submit failure (locked/no funds/peer) → roll back the local scaffold (no half-store). Post-submit timeout → keep the store with status=pending, exit non-zero (resumable via `digstore anchor`).

- **5.3 — `commit` anchors (hard gate, blocks until confirmed).** Read `commands/commit.rs` + the commit path in `ops/store_ops.rs`. After staging computes the new root: `runtime::block_on(anchor.update_root(launcher_id, new_root, &keys, fee))` → block until confirmed → ONLY THEN finalize the local generation (advance `roots.log`). On failure/timeout → abort; `roots.log`/generations untouched. Idempotency: if an update for this root is already pending, reuse it.

- **5.4 — confirmation UX + commands + `[anchor]` config.** Staged human-friendly progress (submitted → mempool → confirming → confirmed) with `--wait-timeout` (default 300s) and `--json` structured states. New `digstore anchor` (resume a pending store) + `digstore anchor status` (query coinset for the active store). Read/write the `[anchor]` table in the store's `config.toml` (`network`, `store_id`/launcher, `coin_id`, `status` pending|confirmed, `last_root`, `last_tx_id`, `confirmed_height`). Persist the current DataStore (or enough to rebuild it) so commits avoid a full lineage sync; fall back to `sync_datastore` for recovery.

- **5.5 — docs + verification.** README anchoring section (init mints, commit anchors, funding, mainnet/coinset, `anchor status`). Run `cargo test -p digstore-chain -p digstore-cli`. Optional `DIGSTORE_E2E`-gated mainnet e2e (never in CI).

**Deferred cleanup (fold in opportunistically, non-blocking):**
- `singleton.rs::build_update` — reorder `select_coins` before `update_store_metadata` (cheaper error path).
- `anchor.rs::update_root` — add a comment re the time-of-check/time-of-push race; add an offline unit test seeding `MockChain` with synthetic lineage.
- `keys.rs`/`WalletKeys` — `chia-bls SecretKey` lacks `Zeroize`; wrap `synthetic_sk`/`master_sk` when the dep supports it (also note in `digstore-security-hardening` memory).
- `singleton.rs` mint test — assert the signature is non-trivial.

**Finish:** when 5.2–5.5 pass, use superpowers:finishing-a-development-branch; before merging delete the throwaway `crates/digstore-chain/examples/anchor_prototype.rs` (Plan Task 0.4).
