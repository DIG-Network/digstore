//! ADVERSARIAL standalone verification of D6 (the core promise).
//!
//! Goal: PROVE (or refute) that a REAL compiled module produced by the genuine
//! store path (init -> add a known file -> commit -> real .wasm) serves its OWN
//! content through `digstore_host::HostRuntime::serve_content` with a merkle
//! proof that genuinely verifies to the trusted root, and that a client can
//! GCM-decrypt the served bytes back to the original file.
//!
//! This test does NOT hand-build a fixture; it drives the actual `store_ops`
//! commit machinery so the .wasm is compiled exactly as `dig commit` would. It
//! also confirms a MISS yields a decoy whose proof does NOT verify to the
//! trusted root.

use std::sync::Arc;

use digstore_cli::context::CliContext;
use digstore_cli::ops::{serve, store_ops};
use digstore_core::config::HostImportsConfig;
use digstore_core::{Bytes32, ChiaBlockRef, ContentResponse, Decode, Decoder, Urn};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};

fn host_deps(store_id: Bytes32, signing_seed: &[u8]) -> HostDeps {
    // §12.2: the host attests with the STORE's host signing key — the same key
    // whose public half the compiler embedded as the trusted key. We reconstruct
    // it from the persisted seed (`signing_key.bin`) so the guest's attestation
    // verification accepts this host (otherwise it serves decoys, correctly).
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

/// The whole D6 promise in one test, with loud printed evidence.
#[test]
fn adversarial_real_module_self_serves_with_verifying_proof() {
    // ---- 1. Build a REAL store: init + add a known file + commit -> real .wasm
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None).unwrap();

    let original: Vec<u8> =
        b"ADVERSARIAL PAYLOAD: prove the module serves its own content. 0123456789".to_vec();
    let f = td.path().join("known.txt");
    std::fs::write(&f, &original).unwrap();
    store_ops::add_path(&ctx, &f, Some("known".into())).unwrap();

    let res = store_ops::commit(&ctx, None, digstore_cli::ops::serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();
    let trusted_root = res.roothash;

    let module = std::fs::read(&res.output_path).unwrap();
    assert!(
        !module.is_empty() && &module[0..4] == b"\0asm",
        "commit must produce a real wasm module"
    );
    eprintln!(
        "[evidence] compiled real .wasm = {} bytes at {}",
        module.len(),
        res.output_path.display()
    );
    eprintln!("[evidence] trusted_root (from commit) = {:?}", trusted_root);

    // ---- 2. Load the REAL module and drive serve_content for the resource
    let signing_seed = std::fs::read(ctx.dig_dir.join("signing_key.bin"))
        .expect("init persisted the host signing seed");
    let mut rt = HostRuntime::new(
        &module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, &signing_seed),
    )
    .expect("host instantiates the real compiled module");

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("known".into()),
    };
    let req = serve::request_for(&urn);
    let resp_bytes = rt
        .serve_content(&req)
        .expect("serve_content returns Ok on the real module");

    // REFUTATION check: empty bytes means the module does NOT serve itself.
    assert!(
        !resp_bytes.is_empty(),
        "REFUTATION: serve_content returned EMPTY — module does NOT self-serve"
    );
    eprintln!(
        "[evidence] serve_content returned {} NON-EMPTY bytes",
        resp_bytes.len()
    );

    // ---- 3. Decode -> ContentResponse, assert proof verifies & roots match
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    let leaf_ok = resp.merkle_proof.leaf == digstore_crypto::sha256(&resp.ciphertext);
    let verifies = resp.merkle_proof.verify();
    let root_matches = resp.merkle_proof.root == trusted_root;
    eprintln!("[evidence] proof.leaf == sha256(ciphertext): {}", leaf_ok);
    eprintln!("[evidence] proof.verify(): {}", verifies);
    eprintln!(
        "[evidence] proof.root == trusted_root: {} (proof.root={:?}, trusted={:?})",
        root_matches, resp.merkle_proof.root, trusted_root
    );

    assert!(
        leaf_ok,
        "REFUTATION: proof.leaf != SHA-256(served ciphertext)"
    );
    assert!(verifies, "REFUTATION: served merkle proof does NOT verify");
    assert!(
        root_matches,
        "REFUTATION: proof.root != trusted root from commit"
    );
    assert_eq!(
        resp.roothash, trusted_root,
        "response roothash must equal trusted root"
    );

    // ---- 4. Client GCM-decrypt the SELF-SERVED bytes -> equals original file
    let key = digstore_cli::ops::client_crypto::derive_decryption_key(&urn, None);
    let lens = store_ops::resource_chunk_lens(&ctx, &trusted_root, "known").unwrap();
    // Split the plain-concatenated ciphertext by chunk lengths and GCM-open each.
    let plan: Vec<usize> = if lens.is_empty() {
        vec![resp.ciphertext.len()]
    } else {
        lens.clone()
    };
    assert_eq!(
        plan.iter().sum::<usize>(),
        resp.ciphertext.len(),
        "chunk lengths must cover the served ciphertext"
    );
    let mut recovered = Vec::new();
    let mut p = 0usize;
    for len in plan {
        let ct = &resp.ciphertext[p..p + len];
        p += len;
        let pt = digstore_crypto::decrypt_chunk(&key, ct)
            .expect("GCM tag must verify on self-served ciphertext");
        recovered.extend_from_slice(&pt);
    }
    eprintln!(
        "[evidence] client GCM-decrypt recovered {} bytes; original {} bytes; equal: {}",
        recovered.len(),
        original.len(),
        recovered == original
    );
    assert_eq!(
        recovered, original,
        "REFUTATION: client decrypt of self-served bytes != original file"
    );

    // ---- 5. A MISS yields a decoy whose proof does NOT verify to trusted root
    let miss_urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("this-resource-does-not-exist".into()),
    };
    let miss_req = serve::request_for(&miss_urn);
    let miss_bytes = rt
        .serve_content(&miss_req)
        .expect("serve_content returns Ok (decoy) on a miss");
    assert!(
        !miss_bytes.is_empty(),
        "decoy must still be a non-empty response"
    );
    let mut dec2 = Decoder::new(&miss_bytes);
    let miss_resp = ContentResponse::decode(&mut dec2).expect("decoy decodes as ContentResponse");
    let decoy_verifies_to_trusted =
        miss_resp.merkle_proof.verify() && miss_resp.merkle_proof.root == trusted_root;
    eprintln!(
        "[evidence] MISS decoy: proof.verify()={}, root==trusted={}, both(=verifies-to-trusted)={}",
        miss_resp.merkle_proof.verify(),
        miss_resp.merkle_proof.root == trusted_root,
        decoy_verifies_to_trusted
    );
    assert_ne!(
        miss_resp.ciphertext, resp.ciphertext,
        "decoy must not return the real resource ciphertext"
    );
    assert!(
        !decoy_verifies_to_trusted,
        "REFUTATION: a MISS decoy verified to the trusted root (gate broken)"
    );

    // Drive the REAL client gate (`verify_chunk_inclusion`) on the decoy: it MUST
    // reject. This is the exact integrity gate `dig cat` uses.
    let gate = digstore_cli::ops::client_crypto::verify_chunk_inclusion(
        &miss_resp.ciphertext,
        &miss_resp.merkle_proof,
        &trusted_root,
    );
    eprintln!(
        "[evidence] MISS decoy rejected by real client gate: {}",
        gate.is_err()
    );
    assert!(
        gate.is_err(),
        "REFUTATION: the real client integrity gate ACCEPTED a miss decoy"
    );

    eprintln!("[evidence] VERDICT: real compiled module self-serves with a verifying proof; miss is a non-verifying decoy.");
}
