//! The serve path must never use a world-known identity (#2553) — and must not
//! demand an identity it does not consume (#2712).
//!
//! These two rules are one rule seen from both sides. #2553's defect was that a
//! missing host signing key silently became `BlsSecretKey::from_seed(&[42u8; 32])`,
//! a value anyone can reproduce from this source. The cure is to stop
//! substituting, NOT to refuse the read: serving committed content consumes no
//! identity (`SPEC.md` §13.6), so refusing cost availability while buying nothing.
//!
//! This file therefore asserts the surviving, positive form of #2553 — the signer
//! is the store's OWN key and never the world-known one — plus #2712's
//! availability rule and the signing-path control that keeps it scoped to reads.
//! Everything is exercised against a REAL store built by the genuine `init` +
//! `add` + `commit` machinery, so the assertions observe the shipped code path
//! rather than a hand-built fixture.
//!
//! The pinned host RNG seed, the third #2553 site, is asserted in `ops::serve`'s
//! own unit test instead, because neither consumer of that RNG is observable from
//! serve output.

use digstore_cli::context::CliContext;
use digstore_cli::ops::{serve, store_ops};
use digstore_core::Urn;

/// The world-known fallback that #2553 removed. Reconstructed here so the test
/// can assert its ABSENCE from real output — a literal comparison against the
/// actual bad value, rather than a proxy for it.
fn world_known_fallback_pubkey() -> [u8; 48] {
    digstore_crypto::bls::SecretKey::from_seed(&[42u8; 32])
        .public_key()
        .to_bytes()
        .0
}

/// A real committed store: returns its context, the compiled module path, and a
/// URN for the one resource it holds.
struct Fixture {
    _td: tempfile::TempDir,
    ctx: CliContext,
    module_path: std::path::PathBuf,
    root: digstore_core::Bytes32,
    urn: Urn,
}

const PAYLOAD: &[u8] = b"fail-closed fixture payload 0123456789";

fn committed_store() -> Fixture {
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None, None, None, None).unwrap();

    let f = td.path().join("known.txt");
    std::fs::write(&f, PAYLOAD).unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();

    let res = store_ops::commit(&ctx, None, serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    Fixture {
        _td: td,
        ctx,
        module_path: res.output_path,
        root: res.roothash,
        urn: Urn {
            chain: "chia".into(),
            store_id,
            root_hash: None,
            resource_key: Some("known".into()),
        },
    }
}

/// CONTROL: with the signing key present the serve path succeeds. Without this
/// the tests below could pass for an unrelated reason (a broken fixture, an
/// uncommitted resource) and prove nothing.
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

/// #2712: with `signing_key.bin` removed, serving must SUCCEED — the read path
/// consumes no identity, so there is nothing to fail closed on.
///
/// This replaces an earlier assertion that serving must ERROR here. That was the
/// wrong expression of #2553: it withheld content whose integrity does not depend
/// on the host at all, and it broke `checkout`, `dev`, `deploy --preview` and
/// `compute_status`, none of which has a network ladder to fall through to.
#[test]
fn serving_succeeds_when_the_host_signing_key_is_missing() {
    let fx = committed_store();
    std::fs::remove_file(fx.ctx.dig_dir.join("signing_key.bin")).unwrap();

    let resp = serve::serve_content(&fx.ctx, &fx.module_path, &fx.urn, fx.root)
        .expect("reading committed content consumes no identity");
    // A retrieval miss returns a DECOY through this same success path (§14.2), so
    // a bare `expect` would be satisfied by a runtime that had silently stopped
    // finding the resource. Assert the ciphertext is the resource's, by checking
    // the plaintext recovered from it.
    let chunk_lens = store_ops::resource_chunk_lens(&fx.ctx, &fx.root, "known").unwrap_or_default();
    let plaintext = digstore_cli::ops::client_crypto::decrypt_and_verify(
        &resp,
        &fx.urn,
        None,
        &fx.root,
        &chunk_lens,
    )
    .expect("the served bytes must be the real resource, not a decoy");
    assert_eq!(plaintext, PAYLOAD);
}

/// #2553, in the form that survives #2712: a proof produced by an intact store is
/// signed by the store's OWN key and NOT by the world-known fallback.
///
/// This is a stronger statement than the refusal it replaces. A refusal only
/// proves the code noticed something was missing; this pins the actual property
/// #2553 cared about — that no output is ever attributed to a key anyone can
/// reproduce from this repository — and it keeps holding on the happy path, where
/// a substituted key would otherwise go unnoticed.
///
/// The expected key is derived HERE from the seed bytes on disk rather than read
/// back through the crate's own `load_host_pubkey`. Asking production code what
/// the key should be would make the assertion circular: a loader that substituted
/// a stand-in would hand the test the same stand-in the signer used, and the
/// comparison would pass. `from_seed` is a crypto primitive, not the code under
/// test, so re-deriving through it is an independent oracle.
#[test]
fn a_proof_is_signed_by_the_stores_own_key_never_the_world_known_fallback() {
    let fx = committed_store();

    let (proof, _root) = serve::serve_proof(&fx.ctx, &fx.module_path, &fx.urn, fx.root)
        .expect("an intact store can sign a proof");

    let seed = std::fs::read(fx.ctx.dig_dir.join("signing_key.bin"))
        .expect("init must persist the host signing key");
    let expected = digstore_crypto::bls::SecretKey::from_seed(&seed)
        .public_key()
        .to_bytes()
        .0;
    assert_eq!(
        proof.node_pubkey.0, expected,
        "the proof must be signed by the store's own host key"
    );
    assert_ne!(
        proof.node_pubkey.0,
        world_known_fallback_pubkey(),
        "the proof must NEVER be signed by from_seed(&[42u8; 32]), which anyone \
         reading this source can reproduce"
    );
}

/// The signing path still FAILS CLOSED, which is what keeps #2712 scoped to reads.
///
/// Signing a proof is an act of attribution, so an absent key must stop it —
/// neither substituting a stand-in nor proceeding without one is available. The
/// error must name the file: an operator handed a bare io error would reasonably
/// go looking for a content problem.
#[test]
fn signing_a_proof_fails_closed_when_the_host_signing_key_is_missing() {
    let fx = committed_store();
    std::fs::remove_file(fx.ctx.dig_dir.join("signing_key.bin")).unwrap();

    let err = serve::serve_proof(&fx.ctx, &fx.module_path, &fx.urn, fx.root)
        .err()
        .expect("a host with no signing key must refuse to SIGN");

    let msg = err.to_string();
    assert!(
        msg.contains("signing key"),
        "error must name the missing signing key, got: {msg}"
    );
}
