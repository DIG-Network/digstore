//! Real mainnet end-to-end anchoring test — init → add → commit → anchor status.
//!
//! !!! THIS SPENDS REAL MAINNET XCH !!!
//!
//! This is the ONLY test in the suite that exercises the real (non-mock)
//! anchoring path against Chia mainnet via coinset.org. It is `#[ignore]`-d AND
//! guarded by the `DIGSTORE_E2E` env var, so it is a no-op unless run manually
//! and deliberately. It is NEVER run in CI.
//!
//! Run it manually, with a funded mainnet wallet mnemonic in the gitignored
//! `.testcredentials` file at the repo root:
//!
//!   DIGSTORE_E2E=1 cargo test -p digstore-cli --test e2e_mainnet -- --ignored
//!
//! It does NOT set `DIGSTORE_ANCHOR_MOCK`: every `init`/`commit` here mints and
//! updates a real singleton on mainnet and blocks on real confirmation, drawing
//! a small fee from the wallet at `.testcredentials`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Path to the repo-root test-wallet mnemonic, RELATIVE to this crate's dir.
/// cargo runs integration tests with the crate dir (`crates/digstore-cli`) as
/// CWD, so `../../.testcredentials` reaches the repo root. The file is read at
/// RUNTIME inside the test body — its contents are never embedded here.
const TESTCREDENTIALS: &str = "../../.testcredentials";

/// Full mainnet flow against the real coinset transport. See the module header:
/// spends real XCH; manual-only.
#[test]
#[ignore = "spends real mainnet XCH; run manually with DIGSTORE_E2E=1 -- --ignored"]
fn mainnet_init_add_commit_anchor_status() {
    // Double safety: even if this test is force-run (e.g. `--ignored`) without
    // the deliberate opt-in, do nothing. NEVER spend XCH by accident.
    if std::env::var("DIGSTORE_E2E").is_err() {
        eprintln!("set DIGSTORE_E2E=1 to run the mainnet e2e (spends real XCH)");
        return;
    }

    // Read the funded mainnet mnemonic at runtime from the gitignored file.
    let mnemonic = std::fs::read_to_string(TESTCREDENTIALS)
        .expect("read .testcredentials (funded mainnet wallet mnemonic)");
    let mnemonic = mnemonic.trim().to_string();

    // Isolate ~/.dig so this run never touches a developer's real wallet config:
    // a throwaway DIGSTORE_HOME holds the encrypted seed + unlock session.
    let project = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let passphrase = "e2e-test-passphrase";

    // A `digstore` invocation in the throwaway project + home. Crucially this
    // does NOT set DIGSTORE_ANCHOR_MOCK — it is the real on-chain path.
    let dig = || {
        let mut cmd = Command::cargo_bin("digstore").unwrap();
        cmd.arg("--dig-dir")
            .arg(project.path().join(".dig"))
            .current_dir(project.path())
            .env("DIGSTORE_HOME", home.path())
            .env("DIGSTORE_PASSPHRASE", passphrase);
        cmd
    };

    // 1. Import the funded mnemonic (encrypts the seed + opens an unlock session).
    dig()
        .args(["seed", "import", "--mnemonic", &mnemonic])
        .assert()
        .success();

    // 2. init: mints the store singleton on mainnet and blocks until confirmed.
    //    store_id := the launcher id. Generous timeout for real confirmation.
    dig()
        .args(["init", "--wait-timeout", "600"])
        .assert()
        .success();

    // 3. add a file to the content root, then commit: pushes the new root on
    //    mainnet and blocks until confirmed before finalizing the generation.
    std::fs::write(project.path().join("readme.txt"), b"hello mainnet\n").unwrap();
    dig()
        .args(["add", "readme.txt", "--key", "readme"])
        .assert()
        .success();
    dig()
        .args([
            "commit",
            "-m",
            "e2e first generation",
            "--wait-timeout",
            "600",
        ])
        .assert()
        .success();

    // 4. anchor status (read-only): the store must report confirmed on-chain.
    dig()
        .args(["anchor", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("confirmed"));
}
