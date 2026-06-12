# Chainstate-in-WASM Implementation Plan (Phase B: chain-verified clone/pull)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make `clone`/`pull` verify the served root against the store singleton's CURRENT on-chain root (read from the chain using the launcher id embedded in the module), failing closed on mismatch or an unreachable chain — closing `SECURITY.md` residual #6.

**Architecture:** Add a `current_root(launcher)` read to `digstore-chain` (sync the singleton over coinset → return its metadata root). In `digstore-cli`'s remote path, after the existing module/head/revocation gates, run a chain-root check: read the module's embedded `ChainState` (Phase A), and require `served_root == on-chain current root`. Offline-testable via the `DIGSTORE_ANCHOR_MOCK` seam (env-configured expected root / unreachable / default-skip).

**Tech Stack:** Rust. `digstore-chain` (`singleton::current_root` over `ChainReads`/`Coinset`), `digstore-cli` (`ops/remote_ops.rs` clone/pull + a chain-verify helper). Spec: `docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md` ("Phase B" section). Builds on Phase A (`ChainState` section + `read_module_chain_state`).

**Conventions (all tasks):** TDD. Conventional commits, SSH-signed, **NO `Co-Authored-By` trailer**. If a build panics about a missing `digstore_guest.wasm`, run `cargo build -p digstore-guest --target wasm32-unknown-unknown --release` first. Never touch mainnet / `.testcredentials`; tests use the `DIGSTORE_ANCHOR_MOCK` seam.

---

## Design decisions (settled — apply as written)

- **Where the chain check runs:** in `clone_from` (after `verify_head_signature` + `check_not_revoked`, before install) and in `pull_from`'s `PullResult::Module` branch (after head sig + revocation, before install). Compare the chain's current root to `remote_root` (clone) / `root` (pull).
- **Backward compat:** if the verified module carries NO `ChainState` (Phase-A-or-older modules), the chain check is SKIPPED (there is no embedded launcher pointer to verify against) — the existing head-signature gate still applies. The chain check is an opt-in stronger guarantee that activates once a module embeds its pointer.
- **Fail closed:** when a `ChainState` IS present: chain root mismatch → `VerificationFailed`; chain unreachable / sync error → fail closed (`VerificationFailed`, never silently install). Do NOT fall back to trusting the served head on a chain error.
- **Launcher source:** use the embedded `ChainState.launcher_id` (it equals the verified `StoreId`; cross-check they're equal and fail closed if not).
- **Mock seam (offline test mechanism):** the CLI chain-verify helper branches on `DIGSTORE_ANCHOR_MOCK`:
  - env set + `DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE=1` → return a fail-closed chain error.
  - env set + `DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT=<hex>` → use that as the "on-chain root" and compare.
  - env set + neither → SKIP the comparison (treat as verified). This keeps the existing `cli_remote_clone_push_pull`/`cli_revoke_tombstone` tests green (they clone modules that now embed a `ChainState` but don't configure a chain root).
  - env NOT set (real) → `Coinset::mainnet()` + `digstore_chain::singleton::current_root(launcher).await`.

---

## File structure

- `crates/digstore-chain/src/singleton.rs` — **modify**: add `pub async fn current_root(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<Bytes32>` (sync the singleton, return its metadata root). One responsibility: read the live on-chain root for a launcher.
- `crates/digstore-cli/src/ops/remote_ops.rs` — **modify**: a `verify_chain_root(module_bytes, store_id, expected_root)` helper + calls in `clone_from` and `pull_from`.
- `crates/digstore-cli/tests/cli_chain_verify.rs` — **create**: clone/pull chain-verify pass + fail-closed + unreachable, via the mock seam.
- `SECURITY.md`, `README.md`, spec — **modify**: residual #6 → closed; note clone/pull are chain-verified.

---

## Task B1: `digstore-chain` — read the live on-chain root for a launcher

**Files:**
- Modify: `crates/digstore-chain/src/singleton.rs`
- Test: same file's test module (reuse the `MockChain` + canned-lineage fixtures already used by the `sync_datastore` test).

> Context: `singleton.rs` already has `pub async fn sync_datastore(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<DataStore>` (walks the lineage over coinset). A synced `DataStore` carries the current metadata root. `Bytes32` here is `chia_protocol::Bytes32` (re-exported via `datalayer_driver`). `crate::coinset::ChainReads` is the read trait; `crate::coinset::mock::MockChain` is the test double.

- [ ] **Step 1: Find the root accessor.** Read `singleton.rs`'s `sync_datastore` and `build_update` to confirm how the current root is read from a `DataStore` (it is the metadata root — locate the exact field path, e.g. `store.info.metadata.root_hash`). Use that exact path in Step 3.

- [ ] **Step 2: Write the failing test.** In the `singleton.rs` test module, mirror the existing `sync_datastore` test (which seeds `MockChain` with a canned launcher→eve lineage and a known root). Add:

```rust
#[tokio::test]
async fn current_root_returns_synced_metadata_root() {
    // Reuse the same MockChain fixture the sync_datastore test builds.
    let (chain, launcher_id, expected_root) = mock_chain_with_lineage();
    let got = current_root(&chain, launcher_id).await.expect("current_root");
    assert_eq!(got, expected_root);
}
```

If the existing sync test inlines its fixture rather than exposing a `mock_chain_with_lineage()` helper, extract that fixture into a small test helper (or inline the same setup here) so both tests share it. The `expected_root` is whatever root the canned lineage's latest singleton commits (the empty/`Bytes32::default()` root for the existing eve-only fixture is fine — assert against the same value the `sync_datastore` test already checks).

- [ ] **Step 3: Run to verify failure.**

Run: `cargo test -p digstore-chain singleton::tests::current_root -- --nocapture`
Expected: FAIL (function `current_root` not found).

- [ ] **Step 4: Implement.** Add to `singleton.rs`:

```rust
/// Read the launcher's current on-chain root by syncing its singleton lineage
/// over `chain` and returning the latest metadata root. Errors (propagated from
/// `sync_datastore`) mean the chain could not be read — callers MUST fail closed.
pub async fn current_root(chain: &dyn ChainReads, launcher_id: Bytes32) -> Result<Bytes32> {
    let store = sync_datastore(chain, launcher_id).await?;
    Ok(store.info.metadata.root_hash) // use the exact field path confirmed in Step 1
}
```

- [ ] **Step 5: Run to verify pass.**

Run: `cargo test -p digstore-chain singleton -- --nocapture`
Expected: PASS.

- [ ] **Step 6: clippy + commit.**

Run: `cargo clippy -p digstore-chain --all-targets` (clean)

```bash
git add crates/digstore-chain/src/singleton.rs
git commit -m "feat(chain): current_root — read a launcher's live on-chain root"
```

---

## Task B2: CLI chain-root verification in clone/pull

**Files:**
- Modify: `crates/digstore-cli/src/ops/remote_ops.rs`
- Test: deferred to Task B3.

> Context: `clone_from` (async) computes `remote_root`, verifies the module (`verify_module_root` → `identity`), `verify_head_signature`, `check_not_revoked`, then installs. `pull_from`'s `PullResult::Module { root, bytes }` branch does the same against `root`. `digstore_core::Bytes32` is the CLI/core id type; `digstore_chain` uses `chia_protocol::Bytes32`. `store_ops::read_module_chain_state(&bytes) -> Result<Option<ChainState>, CliError>` (Phase A) decodes the embedded pointer. `digstore_chain::coinset::Coinset::mainnet()` implements `ChainReads`.

- [ ] **Step 1: Add the chain-verify helper.** In `remote_ops.rs`:

```rust
/// Verify that `expected_root` (the root we are about to install from the remote)
/// equals the store singleton's CURRENT on-chain root, using the launcher pointer
/// embedded in the verified `module`. Fails closed on mismatch or an unreachable
/// chain. If the module carries no `ChainState` (older modules), the check is a
/// no-op (no embedded pointer to verify against) and the head-signature gate
/// remains the authority. Offline-testable via DIGSTORE_ANCHOR_MOCK.
async fn verify_chain_root(
    module: &[u8],
    store_id: &Bytes32,
    expected_root: &Bytes32,
) -> Result<(), CliError> {
    let cs = match store_ops::read_module_chain_state(module)? {
        Some(cs) => cs,
        None => return Ok(()), // no embedded chain pointer; head-sig gate applies
    };
    // The embedded launcher must equal the verified store id (= launcher).
    if cs.launcher_id != *store_id {
        return Err(CliError::VerificationFailed(
            "module ChainState launcher id does not match the store id".into(),
        ));
    }

    // Resolve the live on-chain root (mock seam keeps this offline in tests).
    let onchain: Bytes32 = if std::env::var_os("DIGSTORE_ANCHOR_MOCK").is_some() {
        if std::env::var_os("DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE").is_some() {
            return Err(CliError::VerificationFailed(
                "could not read the store's on-chain root (chain unreachable)".into(),
            ));
        }
        match std::env::var("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT") {
            Ok(hex) => Bytes32::from_hex(&hex)
                .map_err(|_| CliError::Other(anyhow::anyhow!("bad DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT hex")))?,
            Err(_) => return Ok(()), // mock active, no configured root => skip (legacy tests)
        }
    } else {
        // Real path: read the launcher's current root over coinset, fail closed on error.
        let chain = digstore_chain::coinset::Coinset::mainnet();
        let launcher = to_chia_bytes32(store_id); // [u8;32] copy into chia Bytes32
        let root = digstore_chain::singleton::current_root(&chain, launcher)
            .await
            .map_err(|e| CliError::VerificationFailed(format!(
                "could not read the store's on-chain root: {e}"
            )))?;
        from_chia_bytes32(root) // chia Bytes32 -> core Bytes32
    };

    if onchain != *expected_root {
        return Err(CliError::VerificationFailed(format!(
            "served root {} does not match the store's on-chain root {} (chain is the authority)",
            expected_root.to_hex(),
            onchain.to_hex()
        )));
    }
    Ok(())
}

/// core Bytes32 -> chia_protocol Bytes32.
fn to_chia_bytes32(b: &Bytes32) -> chia_protocol::Bytes32 {
    chia_protocol::Bytes32::new(b.0)
}
/// chia_protocol Bytes32 -> core Bytes32.
fn from_chia_bytes32(b: chia_protocol::Bytes32) -> Bytes32 {
    let mut a = [0u8; 32];
    a.copy_from_slice(b.as_ref());
    Bytes32(a)
}
```

Add `chia-protocol` is already a CLI dependency (Phase 5). Confirm `digstore_chain::coinset::Coinset` and `digstore_chain::singleton::current_root` are public (B1 made `current_root` public; `Coinset::mainnet()` is public).

- [ ] **Step 2: Call it in `clone_from`.** After the `check_not_revoked(...)` call and BEFORE the "Install the cloned layout" section, add:

```rust
    // Chain-verified head (SECURITY.md residual #6): the served root must equal the
    // store singleton's current on-chain root. Fail closed on mismatch/unreachable.
    verify_chain_root(&module, &store_id, &remote_root).await?;
```

- [ ] **Step 3: Call it in `pull_from`.** In the `PullResult::Module { root, bytes }` branch, after `check_not_revoked(...)` and BEFORE writing `module_path`, add:

```rust
            verify_chain_root(&bytes, &cfg.store_id, &root).await?;
```

- [ ] **Step 4: Build.** `cargo build -p digstore-cli` — Expected: compiles. `cargo clippy -p digstore-cli --all-targets` — clean. (Tests come in B3.)

- [ ] **Step 5: Commit.**

```bash
git add crates/digstore-cli/src/ops/remote_ops.rs
git commit -m "feat(cli): clone/pull verify served root against the on-chain singleton"
```

---

## Task B3: tests + docs (close residual #6)

**Files:**
- Create: `crates/digstore-cli/tests/cli_chain_verify.rs`
- Modify: `SECURITY.md`, `README.md`, `docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md`

> Context: `tests/common/mod.rs` provides `dig(dir)` (sets `DIGSTORE_ANCHOR_MOCK=1` + seeded session), `store_id_and_root(dir)` (scrapes store_id + newest root hex), `genesis_push_sig`, and `TestServer::start_with_module(store_id_hex, root_hex, public_key, module, genesis_sig)`. The existing `cli_remote_clone_push_pull.rs` shows the full publish→serve→clone pattern: init+add+commit locally, read the compiled module + its pubkey, start a `TestServer`, then `clone` from `http://127.0.0.1:.../stores/<id>` into a fresh dir. Reuse that pattern. NOTE the module produced by `commit` now embeds a `ChainState` whose `launcher_id` == store_id and whose `coinset_url`/etc. are set; its `CurrentRoot` == the committed root.

- [ ] **Step 1: Write the tests.** In `tests/cli_chain_verify.rs`, build a served store once (helper), then exercise the three chain-verify outcomes by setting env on the `clone` command:

```rust
mod common;
use common::*;
use predicates::prelude::*;
use assert_cmd::Command;

// Publish a store to a TestServer and return (server, store_id_hex, root_hex, source_dir).
fn published_store() -> (TestServer, String, String, tempfile::TempDir) {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "x"]).assert().success();
    let (store_id, root) = store_id_and_root(&dir);
    // Load the compiled module + its pubkey (mirror cli_remote_clone_push_pull setup).
    let module_path = store_dir(&dir).join("modules").join(format!("{store_id}-{root}.dig"));
    let module = std::fs::read(&module_path).unwrap();
    let pubkey = host_pubkey(&dir); // helper used by the existing remote tests; reuse it
    let sig = genesis_push_sig(&dir, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pubkey, &module, sig);
    (server, store_id, root, dir)
}

#[test]
fn clone_passes_when_onchain_root_matches() {
    let (server, store_id, root, _src) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT", &root) // chain agrees with served root
        .assert()
        .success();
}

#[test]
fn clone_fails_closed_when_onchain_root_differs() {
    let (server, store_id, _root, _src) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    let bogus = "11".repeat(32);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT", &bogus) // chain says a different root
        .assert()
        .failure()
        .code(5) // VerificationFailed
        .stderr(predicate::str::contains("on-chain root"));
}

#[test]
fn clone_fails_closed_when_chain_unreachable() {
    let (server, store_id, _root, _src) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE", "1")
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("unreachable"));
}
```

If `host_pubkey(&dir)` does not already exist in `common/mod.rs`, add it (read `signing_key.bin`, derive the BLS public key — mirror `genesis_push_sig`'s key load) OR inline the pubkey extraction the existing `cli_remote_clone_push_pull.rs` uses; match whatever that file does to obtain the `[u8;48]` pubkey for `start_with_module`.

- [ ] **Step 2: Run the new tests.**

Run: `cargo test -p digstore-cli --test cli_chain_verify -- --nocapture`
Expected: all three PASS (pass / fail-closed-mismatch / fail-closed-unreachable).

- [ ] **Step 3: Confirm existing remote tests stay green.** The legacy `cli_remote_clone_push_pull`/`cli_revoke_tombstone` tests clone modules that now embed a `ChainState` but set NO chain-root env, so `verify_chain_root` SKIPS (mock active, no configured root). Run:

Run: `cargo test -p digstore-cli --test cli_remote_clone_push_pull --test cli_revoke_tombstone`
Expected: PASS (unchanged).

- [ ] **Step 4: Update SECURITY.md.** Change residual #6 from open to closed: clone/pull now verify the served root against the launcher singleton's current on-chain root (the chain is the authority for the current root); fail closed on mismatch/unreachable. Note the backward-compat caveat (a module with no embedded `ChainState` falls back to the head-signature gate) and that the offline test seam is `DIGSTORE_ANCHOR_MOCK`. Move it from "Residual risks" to the appropriate hardening section, or mark it "Closed (Phase B)".

- [ ] **Step 5: Update README + spec.** README on-chain anchoring section: clone/pull now verify the served root against the store's on-chain singleton (fail closed). Spec `2026-06-11-chainstate-in-wasm-design.md`: mark Phase B implemented.

- [ ] **Step 6: Full workspace verification.**

Run: `cargo test --workspace` — Expected: green (gated `e2e_mainnet` ignored).
Run: `cargo clippy --workspace --all-targets` — clean.
Run: `cargo fmt --check` — clean (run `cargo fmt` if needed).

- [ ] **Step 7: Commit.**

```bash
git add crates/digstore-cli/tests/cli_chain_verify.rs crates/digstore-cli/tests/common/mod.rs SECURITY.md README.md docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md
git commit -m "test+docs: chain-verified clone/pull closes SECURITY residual #6"
```

---

## Self-review (against the spec "Phase B" section)

- **"read launcher_id + network from the module, sync the singleton, require embedded CurrentRoot == on-chain root, fail closed"** — B2's `verify_chain_root` reads the embedded `ChainState`, syncs via `digstore_chain::singleton::current_root` (B1), and requires `served_root == on-chain root`, failing closed on mismatch/unreachable. ✔ (Note: we compare the SERVED root, which the existing gate already proved equals the module's embedded `CurrentRoot` — `id.root == served_root` in clone/pull — so requiring `served_root == on-chain` transitively requires `CurrentRoot == on-chain`.)
- **"closes residual #6"** — B3 Step 4 updates SECURITY.md. ✔
- **"extend the DIGSTORE_ANCHOR_MOCK seam so it's offline-testable"** — B2's env branch (`_CHAIN_ROOT` / `_CHAIN_UNREACHABLE` / default-skip); B3 tests pass/fail/unreachable offline. ✔
- **"coinset endpoint resolution: config/flag, fallback embedded hint, then default"** — the real path uses `Coinset::mainnet()` (the default). NOTE: this plan uses the default endpoint and does NOT yet consult the embedded `coinset_url` hint or a config override for the verification call. That is acceptable for closing residual #6 (api.coinset.org is the default transport) and avoids endpoint-from-untrusted-module risk; wiring config/hint precedence into the verify call is a minor follow-up. Flagged, not silently dropped.
- **Backward compat** — absent `ChainState` ⇒ skip (B2 Step 1); legacy remote tests stay green (B3 Step 3). ✔
- **Placeholder scan** — concrete code in every step; the one lookup (`DataStore` root field path, B1 Step 1) is an explicit investigate-then-use instruction with the likely path named, not a TODO. The `host_pubkey` test helper is specified with a fallback (mirror the existing remote test's extraction).
- **Type consistency** — `verify_chain_root(module:&[u8], store_id:&core Bytes32, expected_root:&core Bytes32)`; B1's `current_root` returns chia `Bytes32`; conversions `to_chia_bytes32`/`from_chia_bytes32` bridge them consistently.
