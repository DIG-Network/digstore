//! Ops-level round trip: commit -> serve (real host instantiate) -> client decrypt + merkle verify.

use digstore_cli::context::CliContext;
use digstore_cli::ops::{client_crypto, serve, store_ops};
use digstore_core::Urn;

fn setup() -> (tempfile::TempDir, CliContext) {
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None).unwrap();
    (td, ctx)
}

#[test]
fn single_chunk_round_trip_through_host_and_client() {
    let (td, ctx) = setup();
    let content = b"the quick brown fox jumps over the lazy dog 1234567890".to_vec();
    let f = td.path().join("doc.txt");
    std::fs::write(&f, &content).unwrap();
    store_ops::add_path(&ctx, &f, Some("doc".into())).unwrap();
    let res = store_ops::commit(&ctx, None).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("doc".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    let out = client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash).unwrap();
    assert_eq!(out, content, "round trip must return original bytes");
}

#[test]
fn multi_chunk_round_trip() {
    let (td, ctx) = setup();
    let mut content = Vec::with_capacity(700 * 1024);
    for i in 0..(700 * 1024) {
        content.push((i % 251) as u8);
    }
    let f = td.path().join("big.bin");
    std::fs::write(&f, &content).unwrap();
    store_ops::add_path(&ctx, &f, Some("big".into())).unwrap();
    let res = store_ops::commit(&ctx, None).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("big".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    let out = client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash).unwrap();
    assert_eq!(
        out, content,
        "multi-chunk round trip must reassemble exactly"
    );
}

#[test]
fn miss_returns_decoy_that_fails_verification() {
    let (td, ctx) = setup();
    let f = td.path().join("doc.txt");
    std::fs::write(&f, b"real content").unwrap();
    store_ops::add_path(&ctx, &f, Some("doc".into())).unwrap();
    let res = store_ops::commit(&ctx, None).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("does-not-exist".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    let err = client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("verification"));
}
