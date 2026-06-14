//! ADVERSARIAL verification of DELEGATED serving (the dighub fix): a module compiled with
//! `init_store(.., host_key_override = Some(P))` embeds an EXTERNAL host key `P` as its sole
//! trusted key (Digstore §12.2), so it is servable ONLY by the node holding the matching secret
//! `S` — NOT by the local store's own (untrusted) signing key.
//!
//! This is the exact dighub serving model: the compile-worker compiles content the RETRIEVAL host
//! (which holds the node BLS secret) will serve. Without embedding the node's public key, the
//! retrieval host fails attestation and `serve_blind` returns decoys for every resource — the root
//! cause of "content won't decrypt in the browser". This test proves the override fixes it:
//!   • served by the delegated key S → HIT: proof verifies to the trusted root + decrypts to the
//!     original bytes;
//!   • served by the local store key (≠ P) → DECOY: the real client integrity gate rejects it.

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

#[test]
fn delegated_host_key_serves_for_node_secret_but_decoys_local() {
    // The DELEGATED serving identity (e.g. the dighub retrieval node): a secret the compile tier
    // never holds — only its PUBLIC half is embedded as the module's trusted key.
    let serving_seed = [0x42u8; 32];
    let serving_pubkey = BlsSecretKey::from_seed(&serving_seed)
        .public_key()
        .to_bytes();

    // ---- 1. Build a store whose TRUSTED host key is the external serving key ----
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None, Some(serving_pubkey)).unwrap();

    let original: Vec<u8> =
        b"DELEGATED-SERVE PAYLOAD: only the node key may release this. 0123456789".to_vec();
    let f = td.path().join("known.txt");
    std::fs::write(&f, &original).unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();
    let res = store_ops::commit(&ctx, None, serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();
    let trusted_root = res.roothash;
    let module = std::fs::read(&res.output_path).unwrap();

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("known".into()),
    };
    let req = serve::request_for(&urn);

    // ---- 2. Served by the DELEGATED key S (pubkey == embedded P) -> HIT + decrypts ----
    let mut rt_node = HostRuntime::new(
        &module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, &serving_seed),
    )
    .expect("host instantiates the compiled module");
    let resp_bytes = rt_node.serve_content(&req).expect("serve_content Ok");
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");
    assert_eq!(
        resp.merkle_proof.leaf,
        digstore_crypto::sha256(&resp.ciphertext),
        "served leaf must equal sha256(served ciphertext)"
    );
    assert!(
        resp.merkle_proof.verify(),
        "delegated-host proof must verify"
    );
    assert_eq!(
        resp.merkle_proof.root, trusted_root,
        "delegated-host proof must root at the trusted root"
    );
    // Client GCM-decrypt the delegated-served bytes -> original.
    let key = digstore_cli::ops::client_crypto::derive_decryption_key(&urn, None);
    let lens = store_ops::resource_chunk_lens(&ctx, &trusted_root, "known").unwrap();
    let plan: Vec<usize> = if lens.is_empty() {
        vec![resp.ciphertext.len()]
    } else {
        lens
    };
    let mut recovered = Vec::new();
    let mut p = 0usize;
    for len in plan {
        let pt = digstore_crypto::decrypt_chunk(&key, &resp.ciphertext[p..p + len])
            .expect("GCM tag verifies on delegated-served ciphertext");
        p += len;
        recovered.extend_from_slice(&pt);
    }
    assert_eq!(
        recovered, original,
        "delegated-served bytes must decrypt to the original"
    );

    // ---- 3. Served by the LOCAL store key (≠ P) -> DECOY rejected by the client gate ----
    let local_seed = std::fs::read(ctx.dig_dir.join("signing_key.bin")).unwrap();
    assert_ne!(
        BlsSecretKey::from_seed(&local_seed)
            .public_key()
            .to_bytes()
            .0,
        serving_pubkey.0,
        "the local signing key must NOT be the trusted serving key (delegation precondition)"
    );
    let mut rt_local = HostRuntime::new(
        &module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, &local_seed),
    )
    .expect("host instantiates the compiled module");
    let decoy_bytes = rt_local
        .serve_content(&req)
        .expect("serve_content Ok (decoy)");
    let mut dec2 = Decoder::new(&decoy_bytes);
    let decoy = ContentResponse::decode(&mut dec2).expect("decoy decodes");
    let gate = digstore_cli::ops::client_crypto::verify_chunk_inclusion(
        &decoy.ciphertext,
        &decoy.merkle_proof,
        &trusted_root,
    );
    assert!(
        gate.is_err(),
        "REFUTATION: an UNTRUSTED host's serve verified to the trusted root (attestation gate broken)"
    );
}
