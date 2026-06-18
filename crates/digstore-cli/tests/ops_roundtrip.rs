//! Ops-level round trip: commit -> serve (real host instantiate) -> client decrypt + merkle verify.

use std::sync::Arc;

use digstore_cli::context::CliContext;
use digstore_cli::ops::{client_crypto, serve, store_ops};
use digstore_core::config::HostImportsConfig;
use digstore_core::{Bytes32, ChiaBlockRef, ContentResponse, Decode, Decoder, Urn};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};

fn setup() -> (tempfile::TempDir, CliContext) {
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None, None, None, None).unwrap();
    (td, ctx)
}

#[test]
fn single_chunk_round_trip_through_host_and_client() {
    let (td, ctx) = setup();
    let content = b"the quick brown fox jumps over the lazy dog 1234567890".to_vec();
    let f = td.path().join("doc.txt");
    std::fs::write(&f, &content).unwrap();
    store_ops::add_path(&ctx, &f, Some("doc".into())).unwrap();
    let res = store_ops::commit(&ctx, None, digstore_cli::ops::serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("doc".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    let lens = store_ops::resource_chunk_lens(&ctx, &res.roothash, "doc").unwrap();
    let out = client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash, &lens).unwrap();
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
    let res = store_ops::commit(&ctx, None, digstore_cli::ops::serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("big".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    let lens = store_ops::resource_chunk_lens(&ctx, &res.roothash, "big").unwrap();
    let out = client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash, &lens).unwrap();
    assert_eq!(
        out, content,
        "multi-chunk round trip must reassemble exactly"
    );
}

/// Build a minimal HostDeps for driving the compiled module directly.
///
/// §12.2: the host attests with the STORE's host signing key — the same key
/// whose public half the compiler embedded as the trusted key. We reconstruct it
/// from the persisted seed (`signing_key.bin`) so the guest's attestation
/// verification accepts this host; otherwise it would (correctly) serve decoys.
fn host_deps(store_id: Bytes32, signing_seed: &[u8]) -> HostDeps {
    let sk = BlsSecretKey::from_seed(signing_seed);
    let pk = sk.public_key().to_bytes();
    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let chain = MockChainSource::new(vec![block.clone()], 1_700_000_000);
    let prover = MockProver::new(prover_sk, prover_pk, block);
    HostDeps {
        store_id,
        bls_secret: sk,
        bls_public: pk,
        clock: Arc::new(FixedClock::new(1_700_000_000)),
        chain: Arc::new(chain),
        prover: Arc::new(prover),
        rng_seed: Some([99u8; 32]),
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}

/// D6: the REAL module a `commit` produces MUST serve itself through
/// `digstore_host::HostRuntime::serve_content` — NOT host-side data-section
/// parsing. Drive the compiled module directly and assert a non-empty
/// `ContentResponse` whose proof verifies, roots match the trusted root, and
/// `leaf == SHA-256(served ciphertext)`.
#[test]
fn commit_module_self_serves_through_host_serve_content() {
    let (td, ctx) = setup();
    let content = b"the module must serve itself through the host runtime".to_vec();
    let f = td.path().join("doc.txt");
    std::fs::write(&f, &content).unwrap();
    store_ops::add_path(&ctx, &f, Some("doc".into())).unwrap();
    let res = store_ops::commit(&ctx, None, digstore_cli::ops::serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    // Root-INDEPENDENT retrieval key (matches commit-time `static_key`).
    let canonical = store_ops::canonical_resource_urn(store_id, "doc");
    let retrieval_key = canonical.retrieval_key();

    let module = std::fs::read(&res.output_path).unwrap();
    let signing_seed = std::fs::read(ctx.dig_dir.join("signing_key.bin"))
        .expect("init persisted the host signing seed");
    let mut rt = HostRuntime::new(
        &module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, &signing_seed),
    )
    .expect("host instantiates the compiled module");

    // Request bytes via the same framing the CLI/guest use.
    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("doc".into()),
    };
    let req = serve::request_for(&urn);
    assert_eq!(serve::request_for(&urn), {
        let mut out = Vec::new();
        out.extend_from_slice(&retrieval_key.0);
        out.push(0); // root_hash: None (root-independent)
        out.push(0); // range
        out.push(0); // jwt
        out.push(0); // window
        out
    });

    let resp_bytes = rt
        .serve_content(&req)
        .expect("serve_content returns Ok on the real module");
    assert!(
        !resp_bytes.is_empty(),
        "serve_content MUST return non-empty bytes — the module must serve itself (D6)"
    );

    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    assert!(
        resp.merkle_proof.verify(),
        "served merkle proof MUST verify"
    );
    assert_eq!(
        resp.merkle_proof.root, res.roothash,
        "proof.root == trusted root"
    );
    assert_eq!(
        resp.merkle_proof.leaf,
        digstore_crypto::sha256(&resp.ciphertext),
        "proof.leaf == SHA-256(served ciphertext)"
    );

    // And the CLI's full client pipeline recovers the original bytes from THIS
    // self-served response.
    let lens = store_ops::resource_chunk_lens(&ctx, &res.roothash, "doc").unwrap();
    let opened =
        client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash, &lens).unwrap();
    assert_eq!(
        opened, content,
        "client decrypt of self-served bytes == original"
    );
}

#[test]
fn miss_returns_decoy_that_fails_verification() {
    let (td, ctx) = setup();
    let f = td.path().join("doc.txt");
    std::fs::write(&f, b"real content").unwrap();
    store_ops::add_path(&ctx, &f, Some("doc".into())).unwrap();
    let res = store_ops::commit(&ctx, None, digstore_cli::ops::serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: Some(res.roothash),
        resource_key: Some("does-not-exist".into()),
    };
    let resp = serve::serve_content(&ctx, &res.output_path, &urn, res.roothash).unwrap();
    // A miss has no manifest entry -> empty chunk_lens; the decoy fails the proof
    // gate before any split/decrypt is attempted.
    let lens =
        store_ops::resource_chunk_lens(&ctx, &res.roothash, "does-not-exist").unwrap_or_default();
    let err =
        client_crypto::decrypt_and_verify(&resp, &urn, None, &res.roothash, &lens).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("verification"));
}
