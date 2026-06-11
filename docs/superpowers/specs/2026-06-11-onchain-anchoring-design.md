# Onchain Anchoring + Seed Management — Design

**Date:** 2026-06-11
**Status:** Approved (design); pending implementation plan
**Scope:** Add Chia-blockchain anchoring and encrypted seed management to the `digstore` CLI, modeled on `dig_library`.

## Summary

Today `digstore` is purely local, content-addressed storage. `init` is offline and instant; each store derives a local `store_id = SHA256(host_pubkey)`.

This feature makes every store an onchain Chia **singleton**:

- **First run** prompts the user to import (or generate) a BIP-39 mnemonic, encrypts it to `~/.dig/seed.enc`, and caches an unlock session.
- **`init`** mints an empty store singleton onchain. The singleton's **launcher id becomes the store_id**. This is a hard gate — if the mint cannot confirm, `init` fails and leaves nothing behind.
- **`commit`** pushes its new root onchain via a singleton `update` transaction and blocks until confirmed before finalizing the local generation. Also a hard gate — local history never advances past the chain.

Anchoring is **mandatory**: there is no offline escape. Network: **mainnet**.

## Decisions (locked)

| Decision | Choice |
|----------|--------|
| Chain access | Build + sign transactions with crates.io `dig-wallet` 2.0 / `dig-store-coin` 2.1 / `datalayer-driver` 3.0 (the crates `dig-chia` wraps). **Broadcast and confirm via coinset.org**, not a direct P2P full-node peer. Stay standalone — no dependency on the `dig_library` workspace. |
| Network | **Mainnet only.** No testnet path in the shipped CLI. Network is hardcoded to mainnet — not configurable. |
| Broadcast transport | coinset.org full-node RPC over HTTPS (`https://api.coinset.org`): `push_tx` to submit signed spend bundles, coin-record / block queries to observe confirmation. No local full node, no P2P peer. |
| Anchor timing | Mandatory. `init` mints; `commit` updates. No `--no-anchor`, no draft mode. |
| Seed entry | Import BIP-39 mnemonic, or generate a new one on first run. |
| Seed at rest | Argon2id (t=3, m=64 MiB, p=4) → AES-256-GCM, in `~/.dig/seed.enc`. Owner-only perms. |
| Unlock | Cached-unlock session file in `~/.dig` with TTL; `DIGSTORE_PASSPHRASE` overrides for non-interactive use. |
| store_id | `store_id := launcher_id`. One identity. The old `SHA256(pubkey)` derivation is dropped. |
| Commit gate | Every commit is a mainnet `update` tx; commit blocks until confirmed before finalizing locally. |

## Open risk to verify in planning

The `dig-chia` source comments that `dig-store-coin` 2.1.0 function signatures *"should be verified against the actual API."* **Before building on it**, confirm `dig-wallet` 2.0.0 + `dig-store-coin` 2.1.0 + `datalayer-driver` 3.0.0 publish on crates.io and compile in this workspace, and pin the real `mint` / `update` / `MintParams` / `UpdateParams` shapes. If the API differs, the anchoring details shift.

**Broadcast transport (critical):** `dig-store-coin::mint` / `update` take a P2P `Peer` and appear to build *and* broadcast in one call. We must broadcast via coinset.org instead. The first planning task verifies whether these crates can either (a) accept a coinset-backed transport, or (b) return the signed `SpendBundle` so we broadcast it ourselves via coinset `push_tx`. If neither, fall back to building/signing the singleton spends at the `datalayer-driver` level and broadcasting via coinset directly. This is the highest-risk unknown — confirm it before committing to the `dig-store-coin` all-in-one path.

Also pin the exact coinset.org endpoints + request/response shapes (`push_tx`, `get_coin_record_by_name`, `get_block_record_by_height` / blockchain state) and any rate limits or auth.

Third planning task: confirm `dig-wallet`'s own key persistence. We feed the mnemonic via `Wallet::from_mnemonic` each run and own the encrypted seed in `~/.dig`; verify dig-wallet does not also persist keys somewhere we must manage or clean up.

## Architecture

New crate **`digstore-chain`** (`crates/digstore-chain`) isolates all blockchain + seed concerns behind a small public API. `digstore-cli` calls only this crate, never the `dig-*` crates directly.

```
digstore-chain
├── seed.rs      # mnemonic import/generate, Argon2id+AES-GCM encrypt/decrypt, ~/.dig I/O
├── unlock.rs    # cached-unlock session (decrypt once, reuse within TTL)
├── wallet.rs    # thin adapter over dig-wallet (from_mnemonic, key derivation, balance)
├── coinset.rs   # coinset.org HTTPS client: push_tx + coin-record/block polling
├── anchor.rs    # ChainAnchor trait + impl: build/sign (dig-store-coin) → broadcast (coinset)
└── config.rs    # ~/.dig global config (coinset url, ttl, default fee)
```

New dependencies: `dig-wallet`, `dig-store-coin`, `datalayer-driver`, `argon2`, `aes-gcm`, `bip39`, `zeroize`, `reqwest` (coinset HTTPS; already in the workspace).

**Mainnet is hardcoded.** The wallet derives mainnet keys/addresses; there is no network selector. Balance and confirmation reads, and all broadcasts, go through coinset.org — the CLI never opens a P2P peer connection.

**`ChainAnchor` trait** abstracts the chain so the hard-gate flows are testable without mainnet:

```rust
#[async_trait]
pub trait ChainAnchor {
    async fn balances(&self, wallet: &Wallet) -> Result<Balances, ChainError>;
    async fn mint_empty_store(&self, wallet: &Wallet, fee: u64) -> Result<MintOutcome, ChainError>;
    async fn update_root(&self, store_id: Bytes32, new_root: Bytes32, wallet: &Wallet, fee: u64)
        -> Result<UpdateOutcome, ChainError>;
    async fn status(&self, store_id: Bytes32) -> Result<AnchorStatus, ChainError>;
}
```

The real impl **builds + signs** the singleton spends with `dig-store-coin` / `datalayer-driver`, then **broadcasts via the coinset.org client** (`coinset.rs`) and **polls coin records / block height** there to observe confirmation. A mock impl drives unit/CLI tests. The coinset HTTP client is itself swappable so transport tests can run against a stub server.

## Seed management

**Global home dir** `~/.dig/` (Windows `%USERPROFILE%\.dig`), distinct from the project-local `.dig/` digstore uses today.

| File | Contents | Perms |
|------|----------|-------|
| `~/.dig/seed.enc` | Encrypted mnemonic: `version(1) ‖ salt(32) ‖ nonce(12) ‖ ciphertext ‖ tag(16)` | owner-only (0600 / Windows ACL) |
| `~/.dig/config.toml` | Global config (network, unlock ttl, default fee) | owner-only |
| `~/.dig/session` | Cached-unlock blob (decrypted seed), valid within TTL | owner-only |

**First run** (any command needing the seed): if `seed.enc` is absent, prompt to **import** a 12/24-word BIP-39 mnemonic (validated) **or generate a new one** (displayed once for backup). Then prompt for a passphrase, encrypt, write `seed.enc`.

**Cached unlock**: after decrypt, cache the decrypted seed in `~/.dig/session` (owner-only) with a TTL (default 1h, configurable). Commands within the TTL skip the passphrase prompt. `DIGSTORE_PASSPHRASE` overrides prompting for CI/non-interactive use. `digstore lock` wipes the session; `digstore seed status` reports unlocked/locked.

**Security note**: caching the decrypted seed on disk is weaker than prompt-every-time — an accepted tradeoff. Mitigations: owner-only perms, TTL expiry, wipe on `lock`. Memory is zeroized (`zeroize`) after use.

**Commands**: `digstore seed import`, `digstore seed generate`, `digstore seed status`, `digstore lock`.

## Flows

### `digstore init [name]` — anchor-first, hard gate

1. **Unlock seed** (first-run import/generate + passphrase → cached).
2. **Build wallet** from the decrypted mnemonic (`Wallet::from_mnemonic`); connect mainnet peer.
3. **Preflight balance** — confirm enough XCH (fee) + DIG (collateral, if `dig-store-coin` requires it). On shortfall: abort with the receive address + shortfall, before any onchain spend.
4. **Mint empty store** — `ChainAnchor::mint_empty_store` (root = EMPTY) → returns the **launcher id**.
5. **`store_id := launcher_id`.** Write the local `.dig/stores/<name>/` layout keyed on this id. The per-store host BLS key is still generated for content signing (contract D6), but no longer derives the id.
6. **Wait for confirmation** (Confirmation UX below). Record `[anchor] status=confirmed`.

Failure semantics turn on whether a launcher id exists yet:

- **Before mint submits** (steps 1–3 fail: locked seed, no funds, peer unreachable, mint rejected before broadcast) → no launcher id, so no store_id is possible. `init` exits non-zero and rolls back the local scaffold — no half-store.
- **After mint submits but before confirmation** (step 6 times out) → the launcher id exists, so the local store is written and kept with `status=pending`. `init` exits non-zero but the store is **resumable** via `digstore anchor`, which polls for confirmation and flips to `confirmed`.

### `digstore commit` — chain-bound, hard gate

1. Stage → compute the new `root_hash` locally.
2. **Onchain `update`** committing the new root to the store singleton (`ChainAnchor::update_root`); **block until confirmed**.
3. **Only on confirmed** → finalize the local generation (advance `roots.log`, write the generation manifest). On failure/timeout → abort; `roots.log` and generations are untouched.

**Idempotency**: a commit retry detects an already-pending `update` tx for the same staged root and reuses its `tx_id` rather than double-spending.

### `digstore anchor` / `digstore anchor status`

- `digstore anchor` — resume a `pending` store (mint submitted, confirmation not yet observed): polls the chain and flips to `confirmed`. A fully-failed init leaves no store (rolled back) — re-run `init` instead.
- `digstore anchor status` — query live chain state for the active store.

## Confirmation UX

Blocking wait with a staged, human-friendly indicator:

```
⛓  Anchoring on Chia mainnet…
   ✓ submitted        tx 0xab12…f9
   ⏳ in mempool       (waiting for a block)
   ⏳ confirming       2/3 blocks
   ✓ confirmed         height 5,012,233  ·  42s
```

Confirmation is observed by polling coinset.org (coin record spent/created + current block height) on an interval until the target depth is reached.

- `--wait-timeout` (default 5 min). On timeout: leave `status=pending`, tell the user it will confirm in the background, and to check `digstore anchor status`. (For `commit`, a timeout aborts finalization — local history stays behind the chain until confirmation is observed.)
- Status surfaces wherever store status is shown: `Anchor: ✓ confirmed (mainnet, store 0x…, height …)` / `⏳ pending (3m)` / `✗ failed`.
- `--json` emits structured state transitions for scripting.

## Data model

**Global** `~/.dig/config.toml`:

```toml
coinset_url = "https://api.coinset.org"  # broadcast + confirmation endpoint
unlock_ttl  = 3600                        # seconds
fee         = 0                           # default tx fee (mojos); 0 = auto/estimate
```

Network is not configurable — mainnet is hardcoded. `coinset_url` is overridable only so a different coinset-compatible mainnet endpoint can be pointed at if api.coinset.org is unavailable.

**Per-store** `config.toml` gains an `[anchor]` table:

```toml
[anchor]
network          = "mainnet"
store_id         = "0x…"      # == launcher id == the store identity
coin_id          = "0x…"      # current singleton coin
status           = "confirmed" # pending | confirmed  (failed inits roll back, leaving no store)
last_root        = "0x…"       # last root anchored onchain
last_tx_id       = "0x…"
confirmed_height = 0
```

There is no separate local store_id — one identity, the launcher.

## Error handling

All chain/seed errors map to `CliError` variants with a human message and a `help:` hint (matching the existing error style):

| Variant | Trigger | UX |
|---------|---------|-----|
| `NoSeed` | `seed.enc` absent | Run first-run import/generate |
| `BadPassphrase` | decrypt fails | Re-prompt (bounded retries) then abort |
| `InvalidMnemonic` | BIP-39 validation fails | Report the reason |
| `InsufficientFunds { need, have, address }` | preflight | Print receive address + shortfall |
| `PeerUnreachable` | can't reach mainnet | Bounded retry/backoff, clear message |
| `MintFailed` / `UpdateFailed` | chain rejects | Surface the chain reason |
| `ConfirmTimeout` | not confirmed in time | Leave `pending`, resumable |
| `Locked` | session expired | Re-unlock |

Hard-gate cleanup: init rolls back the local scaffold on failure; commit leaves `roots.log`/generations untouched on failure.

## Testing

- **Unit**: seed encrypt/decrypt round-trip; BIP-39 test vectors; Argon2id params; session TTL expiry; `[anchor]` config serde; error→message mapping; confirmation-status formatting.
- **Mock chain** (`ChainAnchor` mock): init hard-gate, commit-blocks-until-confirmed, timeout→pending, retry/idempotency — all offline and deterministic.
- **Coinset transport** (stub HTTP server, e.g. `wiremock`): `push_tx` success/reject, coin-record polling, block-height progression, malformed responses, timeouts/retries — without touching the real network.
- **Manual mainnet e2e** (not in CI): there is no testnet path, so a real end-to-end mint/update runs only manually on mainnet behind `#[ignore]` + a `DIGSTORE_E2E` guard, and spends real XCH. Document the cost; never wire it into automated CI.
- **CLI** (`assert_cmd`): command wiring; non-interactive unlock via `DIGSTORE_PASSPHRASE`; `anchor` / `anchor status` output; `--json` shape.

## Out of scope (this spec)

- Mirror coins, collateral top-up/reclaim beyond what `mint` requires, epoch/L2 anchoring (`l2-anchor`).
- Multi-account / multiple mnemonics. One global seed.
- Key rotation.

## Verification spike results (2026-06-11) — RESOLVES the top risk; CHANGES the approach

The crates publish at the assumed versions: `dig-store-coin 2.1.0`, `dig-wallet 2.0.0`, `datalayer-driver 3.0.0`. coinset.org exposes the standard Chia Full Node RPC over HTTPS POST with no TLS cert (`push_tx`, `get_coin_record_by_name`, `get_coin_records_by_puzzle_hash`, blockchain state / block-record reads at `https://api.coinset.org/<endpoint>`).

**Critical finding — `dig-store-coin` is NOT usable for coinset-only broadcast.** Its `MintParams` carries `ssl_cert_path` / `ssl_key_path` / `network`; `dig_store_coin::mint` (and `update`) connect to a Chia full node over **TLS P2P** and broadcast there. `datalayer-driver`'s connection API is likewise P2P (`connect_peer(network, tls_connector, SocketAddr)`, `broadcast_spend_bundle(peer, …)`). coinset.org is HTTP RPC, not a P2P peer — so the thin `dig_store_coin::mint(params, peer, wallet)` wrapper assumed earlier in this spec **cannot be used**.

**Corrected approach — build at the `datalayer-driver` level, broadcast via coinset (this is fallback (c) from the original risk note).** `datalayer-driver` exposes pure builder functions that take no `Peer`:
- `mint_store(minter_synthetic_key, selected_coins: Vec<Coin>, root_hash, label, description, bytes, size_proof, owner_puzzle_hash, delegated_puzzles, fee) -> SuccessResponse`
- `update_store_metadata(...)` / `update_store_ownership(...)`
- `sign_coin_spends(...)`, `select_coins(...)`, `add_fee(...)`, `spend_bundle_to_hex(...)`, `get_coin_id(...)`, `get_mainnet_genesis_challenge()` (const)

`anchor.rs` therefore becomes a bespoke pipeline, NOT a wrapper:
1. Derive the wallet synthetic key + owner puzzle hash from the seed (datalayer-driver `master_*`/`*_to_puzzle_hash` helpers; `dig-wallet` may be used for mnemonic→master-key only).
2. Fetch the wallet's unspent XCH coins from **coinset** (`get_coin_records_by_puzzle_hash`) and `select_coins` for the fee.
3. `mint_store(root = EMPTY, …)` for init / `update_store_metadata(new_root, …)` for commit → builds coin spends + the launcher/store id.
4. `sign_coin_spends(...)` with the synthetic secret key.
5. Serialize and broadcast via **coinset `push_tx`** (our `coinset.rs` client), NOT `broadcast_spend_bundle`.
6. Poll **coinset** (`get_coin_record_by_name` + peak height) for confirmation.

**New hardest task for Plan 2:** singleton **sync over coinset** — to build `commit`'s `update`, we must find the current unspent singleton coin for a launcher id by following its lineage via coinset coin-record reads (datalayer-driver's `sync_store_from_launcher_id` is Peer-based and unusable). This is the largest single risk in subsystem 2.

**Scope impact:** subsystem 2 is a custom Chia singleton client over coinset HTTP — materially larger and riskier than "wrap dig-store-coin." The `ChainAnchor` trait and the build/sign-then-broadcast split still hold; only the implementation behind it changes (datalayer-driver low-level + coinset, no P2P peer, no `dig-store-coin`).
