//! The serve path must FAIL CLOSED, not fall back to a world-known default.
//!
//! Two independent fail-open sites lived in `ops::serve` (issue #2553).
//! This file covers the first: a missing host signing key silently became
//! `BlsSecretKey::from_seed(&[42u8; 32])`, a value anyone can reproduce from the
//! source. It is exercised against a REAL store built by the genuine `init` +
//! `add` + `commit` machinery, so the assertions observe the shipped code path
//! rather than a hand-built fixture.
//!
//! The second — the pinned host RNG seed — is asserted in `ops::serve`'s own
//! unit test instead, because neither consumer of that RNG is observable from
//! serve output.

use digstore_cli::context::CliContext;
use digstore_cli::ops::{serve, store_ops};
use digstore_core::Urn;

/// A real committed store: returns its context, the compiled module path, and a
/// URN for the one resource it holds.
struct Fixture {
    _td: tempfile::TempDir,
    ctx: CliContext,
    module_path: std::path::PathBuf,
    urn: Urn,
}

fn committed_store() -> Fixture {
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None, None, None, None).unwrap();

    let f = td.path().join("known.txt");
    std::fs::write(&f, b"fail-closed fixture payload 0123456789").unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();

    let res = store_ops::commit(&ctx, None, serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    Fixture {
        _td: td,
        ctx,
        module_path: res.output_path,
        urn: Urn {
            chain: "chia".into(),
            store_id,
            root_hash: None,
            resource_key: Some("known".into()),
        },
    }
}

/// CONTROL: with the signing key present the serve path succeeds. Without this
/// the "missing key fails" test below could pass for an unrelated reason (a
/// broken fixture, an uncommitted resource) and prove nothing.
#[test]
fn serving_succeeds_while_the_host_signing_key_is_present() {
    let fx = committed_store();
    assert!(
        fx.ctx.dig_dir.join("signing_key.bin").exists(),
        "init must persist the host signing key"
    );
    serve::serve_content_raw(&fx.ctx, &fx.module_path, &fx.urn)
        .expect("a store with its signing key serves normally");
}

/// FAIL CLOSED: with `signing_key.bin` removed, serving must ERROR rather than
/// silently attest with a world-known key baked into the source. The fallback
/// key is reproducible by anyone reading this repository, so a host using it
/// carries no identity at all — the operator must learn that the key is gone,
/// not be handed a degraded serve that looks like a content problem.
#[test]
fn serving_fails_closed_when_the_host_signing_key_is_missing() {
    let fx = committed_store();
    let key_path = fx.ctx.dig_dir.join("signing_key.bin");
    std::fs::remove_file(&key_path).unwrap();

    let err = serve::serve_content_raw(&fx.ctx, &fx.module_path, &fx.urn)
        .expect_err("a host with no signing key must refuse to serve");

    // Observe the REASON, not merely the failure: the error must name the
    // signing key, otherwise this test would also pass on an unrelated break.
    let msg = err.to_string();
    assert!(
        msg.contains("signing key"),
        "error must name the missing signing key, got: {msg}"
    );
}
