//! `cat` reads without a store identity; signing still requires one (#2712).
//!
//! Reading DIG content never needs an account or a key (`SPEC.md` §13.6/§14), so a
//! store whose `signing_key.bin` / `trusted_keys.json` is missing or unreadable
//! must still serve the content it has already committed. Before this, the read
//! path loaded both files and aborted, which also pre-empted the client→node
//! ladder: `cat`'s local leg returned `Err` rather than the "not here, try the
//! network" signal, so the ladder was never reached.
//!
//! These are the command-level twins of the unit tests in `ops::serve::tests`.
//! They exist separately because the unit tests drive `serve_content_raw`
//! directly, and the property users actually depend on is the exit status of a
//! whole `cat` invocation.

mod common;
use common::{dig, store_id_and_root, tmp_dig};

use std::path::Path;

/// Delete every file carrying the store's identity, and assert they were really
/// there — a fixture that silently deleted nothing would make every assertion
/// below vacuous.
fn destroy_identity(dig_dir: &Path) {
    let store_dir = dig_dir.join("stores").join("default");
    let mut removed = 0;
    for name in ["signing_key.bin", "trusted_keys.json"] {
        let p = store_dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
            removed += 1;
        }
    }
    assert_eq!(
        removed,
        2,
        "fixture must actually remove BOTH identity files from {}; if the layout \
         moved, this test is no longer exercising a store without an identity",
        store_dir.display()
    );
}

/// Commit a one-file store and return its URN.
fn committed_store(dir: &tempfile::TempDir, content: &[u8]) -> String {
    let f = dir.path().join("doc.txt");
    std::fs::write(&f, content).unwrap();
    dig(dir).arg("init").assert().success();
    dig(dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(dir).args(["commit"]).assert().success();
    let (store_id, root) = store_id_and_root(dir);
    format!("urn:dig:chia:{store_id}:{root}/doc")
}

#[test]
fn cat_serves_committed_content_after_the_store_identity_is_destroyed() {
    let dir = tmp_dig();
    let content = b"readable without an identity";
    let urn = committed_store(&dir, content);

    // Control: the intact store returns the plaintext. Without this, a store that
    // had stopped serving for an unrelated reason could not be told apart from the
    // regression under test.
    let intact = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(intact.status.success());
    assert_eq!(intact.stdout, content);

    destroy_identity(&dir.path().join(".dig"));

    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(
        out.status.success(),
        "cat must still serve when the store has no identity; it consumes none. \
         stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Assert the PLAINTEXT, not merely a zero exit. A retrieval miss returns a
    // decoy through the same success path (§14.2), so a runtime that had quietly
    // stopped finding the resource would satisfy an exit-status-only check.
    assert_eq!(
        out.stdout, content,
        "an identity-less read must return the real plaintext, never a decoy"
    );
}

#[test]
fn cat_verify_proof_still_refuses_without_a_signing_key() {
    let dir = tmp_dig();
    let urn = committed_store(&dir, b"proof needs a signer");

    // Control: with the identity intact, the proof verifies.
    dig(&dir)
        .args(["cat", "--verify-proof", &urn])
        .assert()
        .success();

    destroy_identity(&dir.path().join(".dig"));

    // This is the control that keeps the relaxation honest. Identity became
    // optional on the READ path only: `--verify-proof` signs an execution proof,
    // and signing is an act of attribution, so it must refuse. Without this
    // assertion the test above is satisfied equally by "identity optional on
    // reads" and by "identity optional everywhere" — the second being a security
    // regression.
    let out = dig(&dir)
        .args(["cat", "--verify-proof", &urn])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "signing a proof REQUIRES an identity and must refuse without one"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The refusal must name the file that is gone. The released 0.23.0 binary
    // instead substituted a world-known fallback key and failed several layers
    // later with `NodeKeyNotAttested(b145dfcb…)` plus a hint blaming the CONTENT
    // ("the store data was tampered with") — a true statement about the wrong
    // subject, which sends an operator looking for corruption that is not there.
    assert!(
        stderr.contains("signing_key.bin"),
        "the refusal must name the missing identity file rather than blame the \
         content: {stderr}"
    );
}
