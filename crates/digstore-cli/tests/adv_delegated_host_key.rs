//! Verification that dighub content is servable by ANY node (the public-serve model).
//!
//! The host-key attestation gate was REMOVED (digstore-guest `GateConfig::from_embedded`
//! `require_attestation = false`): `serve_blind` no longer gates real content on the serving host's
//! BLS key being in the module's embedded trusted set, so content is servable by ANY node — anonymous
//! or otherwise — with a single, stable program_hash network-wide (re-keying per node would change the
//! program_hash, which is the execution-proof identity and MUST be identical on every node).
//!
//! This test proves the new contract: a module compiled with one host key override is served as REAL,
//! verifying, decryptable content by BOTH the matching ("delegated") key AND a completely different
//! (formerly "untrusted") local key. The oblivious property is unaffected — a resource MISS still
//! returns an indistinguishable non-verifying decoy (covered by `digstore-host`'s `dighost_serve`).

use std::sync::Arc;

use digstore_cli::context::CliContext;
use digstore_cli::ops::{serve, store_ops};
use digstore_core::config::HostImportsConfig;
use digstore_core::{Bytes32, ChiaBlockRef, ContentResponse, Decode, Decoder, Urn};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};

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

/// Serve `req` from `module` using a host whose BLS identity is derived from `signing_seed`, and
/// assert the response is REAL content: the proof's leaf is `sha256(ciphertext)`, the proof verifies
/// to `trusted_root`, and the ciphertext GCM-decrypts (per the resource chunk plan) to `original`.
#[allow(clippy::too_many_arguments)]
fn assert_serves_real(
    module: &[u8],
    store_id: Bytes32,
    urn: &Urn,
    trusted_root: Bytes32,
    chunk_lens: &[usize],
    signing_seed: &[u8],
    original: &[u8],
    who: &str,
) {
    let req = serve::request_for(urn);
    let mut rt = HostRuntime::new(
        module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, signing_seed),
    )
    .expect("host instantiates the compiled module");
    let resp_bytes = rt.serve_content(&req).expect("serve_content Ok");
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");
    assert_eq!(
        resp.merkle_proof.leaf,
        digstore_crypto::sha256(&resp.ciphertext),
        "{who}: served leaf must equal sha256(served ciphertext)"
    );
    assert!(
        resp.merkle_proof.verify(),
        "{who}: served proof must verify"
    );
    assert_eq!(
        resp.merkle_proof.root, trusted_root,
        "{who}: served proof must root at the trusted root"
    );
    let key = digstore_cli::ops::client_crypto::derive_decryption_key(urn, None);
    let plan: Vec<usize> = if chunk_lens.is_empty() {
        vec![resp.ciphertext.len()]
    } else {
        chunk_lens.to_vec()
    };
    let mut recovered = Vec::new();
    let mut p = 0usize;
    for len in plan {
        let pt = digstore_crypto::decrypt_chunk(&key, &resp.ciphertext[p..p + len])
            .expect("GCM tag verifies on served ciphertext");
        p += len;
        recovered.extend_from_slice(&pt);
    }
    assert_eq!(
        recovered, original,
        "{who}: served bytes must decrypt to the original"
    );
}

#[test]
fn any_host_serves_real_content_with_attestation_disabled() {
    // A host key embedded as the module's "trusted" key (the historical delegated-serve identity).
    // With attestation disabled it carries no special privilege — it must serve exactly like any
    // other host key.
    let serving_seed = [0x42u8; 32];
    let serving_pubkey = BlsSecretKey::from_seed(&serving_seed)
        .public_key()
        .to_bytes();

    // ---- Build a store (embedding `serving_pubkey` as the trusted key, as the compile-worker did) --
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None, Some(serving_pubkey)).unwrap();

    let original: Vec<u8> =
        b"PUBLIC-SERVE PAYLOAD: any node may release this. 0123456789abcdef".to_vec();
    let f = td.path().join("known.txt");
    std::fs::write(&f, &original).unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();
    let res = store_ops::commit(&ctx, None, serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();
    let trusted_root = res.roothash;
    let module = std::fs::read(&res.output_path).unwrap();
    let chunk_lens = store_ops::resource_chunk_lens(&ctx, &trusted_root, "known").unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("known".into()),
    };

    // ---- 1. Served by the embedded ("delegated") key -> REAL content ----
    assert_serves_real(
        &module,
        store_id,
        &urn,
        trusted_root,
        &chunk_lens,
        &serving_seed,
        &original,
        "embedded-key host",
    );

    // ---- 2. Served by the LOCAL store key (≠ embedded key) -> ALSO REAL content (any node serves) --
    let local_seed = std::fs::read(ctx.dig_dir.join("signing_key.bin")).unwrap();
    assert_ne!(
        BlsSecretKey::from_seed(&local_seed)
            .public_key()
            .to_bytes()
            .0,
        serving_pubkey.0,
        "the local signing key must differ from the embedded key (so this exercises a non-embedded host)"
    );
    assert_serves_real(
        &module,
        store_id,
        &urn,
        trusted_root,
        &chunk_lens,
        &local_seed,
        &original,
        "local/non-embedded host",
    );

    // ---- 3. Served by a THIRD, totally unrelated key -> ALSO REAL (anonymous node serves) ----
    assert_serves_real(
        &module,
        store_id,
        &urn,
        trusted_root,
        &chunk_lens,
        &[0xABu8; 32],
        &original,
        "anonymous host",
    );
}
