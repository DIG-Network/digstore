//! Artifact 3 (`dighost`) integration tests.
//!
//! These drive the SAME blind-serve path the `dighost` binary uses
//! (`digstore_host::serve_blind`) over a REAL compiled fixture module, loaded
//! through the `object_store` abstraction (InMemory + LocalFileSystem). They
//! prove:
//!   1. serve-by-retrieval-key over object_store returns a non-empty
//!      `ContentResponse` whose merkle proof verifies to the trusted root,
//!   2. a miss returns an indistinguishable decoy whose proof does NOT verify,
//!   3. the served bytes are CIPHERTEXT (host is blind — bytes != plaintext),
//!   4. an `s3://bucket/key` URL is fetched through object_store's S3-shaped
//!      path (exercised against InMemory, since the AmazonS3 wiring is unit
//!      tested in the binary without a live bucket).

use std::sync::Arc;

use digstore_cli::context::CliContext;
use digstore_cli::ops::store_ops;
use digstore_core::{ContentResponse, Decode, Decoder, Urn};
use digstore_host::{serve_blind, BlindServeConfig};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

/// The known plaintext committed by the fixture.
const ORIGINAL: &[u8] =
    b"ARTIFACT-3 dighost: blind host serves ciphertext by retrieval key. 0123456789";

/// Build a REAL store (init -> add a known file -> commit) and return the
/// compiled module bytes, the store id, the trusted root, the host signing seed,
/// and the 32-byte retrieval key for resource "known".
struct Fixture {
    module: Vec<u8>,
    trusted_root: digstore_core::Bytes32,
    seed: Vec<u8>,
    store_id: digstore_core::Bytes32,
    retrieval_key: [u8; 32],
}

fn build_fixture() -> (tempfile::TempDir, Fixture) {
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None).unwrap();

    let f = td.path().join("known.txt");
    std::fs::write(&f, ORIGINAL).unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();

    let res = store_ops::commit(&ctx, None).unwrap();
    let store_id = ctx.find_store_id().unwrap();
    let trusted_root = res.roothash;
    let module = std::fs::read(&res.output_path).unwrap();
    assert!(
        !module.is_empty() && &module[0..4] == b"\0asm",
        "commit must produce a real wasm module"
    );
    let seed = std::fs::read(ctx.dig_dir.join("signing_key.bin")).unwrap();

    // Root-INDEPENDENT canonical URN -> the retrieval key the compiler stored.
    let canonical = store_ops::canonical_resource_urn(store_id, "known");
    let retrieval_key = canonical.retrieval_key().0;

    (
        td,
        Fixture {
            module,
            trusted_root,
            seed,
            store_id,
            retrieval_key,
        },
    )
}

/// Fetch module bytes from any object_store (mirrors the binary's fetch path).
async fn fetch(store: &dyn ObjectStore, key: &ObjPath) -> Vec<u8> {
    store
        .get(key)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec()
}

/// Serve over a given object_store and return the verbatim served bytes.
fn serve_over_store(
    store: Arc<dyn ObjectStore>,
    key: ObjPath,
    fx: &Fixture,
    rk: [u8; 32],
) -> Vec<u8> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let bytes = rt.block_on(async { fetch(store.as_ref(), &key).await });
    assert_eq!(
        bytes, fx.module,
        "object_store returned the exact module bytes"
    );
    let cfg = BlindServeConfig::from_seed(fx.store_id, &fx.seed);
    serve_blind(&bytes, &rk, cfg).expect("serve_blind ok")
}

#[test]
fn inmemory_serve_by_retrieval_key_verifies_to_root() {
    let (_td, fx) = build_fixture();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let key = ObjPath::from("storeid-root.wasm");
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            store.put(&key, fx.module.clone().into()).await.unwrap();
        });
    }

    let served = serve_over_store(store.clone(), key.clone(), &fx, fx.retrieval_key);
    assert!(!served.is_empty(), "served bytes must be non-empty");

    let mut dec = Decoder::new(&served);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    assert!(resp.merkle_proof.verify(), "served proof must verify");
    assert_eq!(
        resp.merkle_proof.root, fx.trusted_root,
        "served proof roots at the trusted root"
    );
    assert_eq!(resp.roothash, fx.trusted_root);

    // Host is BLIND: served ciphertext must NOT equal the known plaintext.
    assert_ne!(
        resp.ciphertext.as_slice(),
        ORIGINAL,
        "served bytes must be ciphertext, not plaintext"
    );
    // And the original plaintext must not appear verbatim inside the envelope.
    assert!(
        served.windows(ORIGINAL.len()).all(|w| w != ORIGINAL),
        "plaintext must not appear in the served envelope"
    );
}

#[test]
fn localfs_serve_by_retrieval_key_verifies_to_root() {
    let (_td, fx) = build_fixture();
    let dir = tempfile::tempdir().unwrap();
    let module_path = dir.path().join("storeid-root.wasm");
    std::fs::write(&module_path, &fx.module).unwrap();

    let store: Arc<dyn ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let key = ObjPath::from("storeid-root.wasm");

    let served = serve_over_store(store, key, &fx, fx.retrieval_key);
    let mut dec = Decoder::new(&served);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");
    assert!(resp.merkle_proof.verify());
    assert_eq!(resp.merkle_proof.root, fx.trusted_root);
    assert_ne!(resp.ciphertext.as_slice(), ORIGINAL);
}

#[test]
fn miss_returns_nonverifying_decoy() {
    let (_td, fx) = build_fixture();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let key = ObjPath::from("m.wasm");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { store.put(&key, fx.module.clone().into()).await.unwrap() });

    // A retrieval key for a resource that does not exist.
    let miss_urn = Urn {
        chain: "chia".into(),
        store_id: fx.store_id,
        root_hash: None,
        resource_key: Some("nope-not-here".into()),
    };
    let miss_rk = miss_urn.retrieval_key().0;

    let served = serve_over_store(store, key, &fx, miss_rk);
    assert!(
        !served.is_empty(),
        "decoy must still be non-empty (same wire shape)"
    );
    let mut dec = Decoder::new(&served);
    let resp = ContentResponse::decode(&mut dec).expect("decoy decodes as ContentResponse");

    let verifies_to_trusted =
        resp.merkle_proof.verify() && resp.merkle_proof.root == fx.trusted_root;
    assert!(
        !verifies_to_trusted,
        "a MISS decoy must NOT verify to the trusted root"
    );
}

#[test]
fn s3_url_path_routes_through_object_store() {
    // Exercise the s3:// fetch shape against InMemory: the key parsed from an
    // s3://bucket/key URL is used verbatim as the object_store path. (Live
    // AmazonS3 builder construction is unit-tested in the binary.)
    let (_td, fx) = build_fixture();

    // Parse s3://bucket/key the same way the binary does.
    let url = "s3://my-store-bucket/storeid-root.wasm";
    let rest = url.strip_prefix("s3://").unwrap();
    let (bucket, key_str) = rest.split_once('/').unwrap();
    assert_eq!(bucket, "my-store-bucket");
    assert_eq!(key_str, "storeid-root.wasm");

    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let key = ObjPath::from(key_str);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { store.put(&key, fx.module.clone().into()).await.unwrap() });

    let served = serve_over_store(store, key, &fx, fx.retrieval_key);
    let mut dec = Decoder::new(&served);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");
    assert!(resp.merkle_proof.verify());
    assert_eq!(resp.merkle_proof.root, fx.trusted_root);
}
