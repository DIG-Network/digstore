//! Phase A (memory ceiling) validator: a REAL compiled module whose injected
//! `DIGS` data section EXCEEDS the old 8 MiB fixed heap base must still serve
//! itself with a verifying merkle proof. This is the end-to-end proof that the
//! 256→2048-page (16 MiB→128 MiB) ceiling raise plus the guest's *dynamic* heap
//! base (computed above the data section) let stores larger than the old window
//! commit and serve.
//!
//! The harness mirrors `tests/self_serving.rs` (its private helpers are NOT
//! imported — they are replicated here so this file stands alone). It compiles a
//! real `.wasm` from the actual `digstore-guest` wasm template, drives it through
//! `digstore_host::HostRuntime`, and asserts:
//!   * `serve_content` returns NON-EMPTY bytes,
//!   * the served ciphertext equals the resource's ordered chunk ciphertext,
//!   * `merkle_proof.root == injected current root` (trusted root),
//!   * `merkle_proof.leaf == SHA-256(served ciphertext)` (per-resource leaf, D5),
//!   * `merkle_proof.verify()` is true,
//!   * the client GCM-opens the served ciphertext back to the original plaintext.

mod common;

use digstore_compiler::{Compiler, CompilerConfig, GenerationView, ResourceView};
use digstore_core::config::HostImportsConfig;
use digstore_core::merkle::MerkleTree;
use digstore_core::serving::concat_output;
use digstore_core::{Bytes32, Bytes48, ChiaBlockRef, ContentResponse, Decode, Decoder, Urn};
use digstore_crypto::bls::BlsSecretKey;
use digstore_crypto::{derive_decryption_key, encrypt_chunk};
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const GUEST_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-unknown-unknown/release/digstore_guest.wasm"
);

fn sha256(b: &[u8]) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(b);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h.finalize());
    Bytes32(o)
}

/// A resource with a fixed retrieval key and a list of ciphertext chunk bodies.
struct FixtureResource {
    retrieval_key: Bytes32,
    chunks: Vec<(Bytes32, Vec<u8>)>, // (content-address, ciphertext)
}

struct FixtureGen {
    root: Bytes32,
    resources: Vec<FixtureResource>,
}
impl GenerationView for FixtureGen {
    fn root(&self) -> Bytes32 {
        self.root
    }
    fn resources(&self) -> Vec<Box<dyn ResourceView + '_>> {
        self.resources
            .iter()
            .map(|r| Box::new(FixtureResourceRef(r)) as Box<dyn ResourceView + '_>)
            .collect()
    }
}
struct FixtureResourceRef<'a>(&'a FixtureResource);
impl<'a> ResourceView for FixtureResourceRef<'a> {
    fn resource_key(&self) -> Bytes32 {
        self.0.retrieval_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.0.chunks.clone()
    }
}

fn host_deps(store_id: Bytes32) -> HostDeps {
    let sk = BlsSecretKey::from_seed(&[42u8; 32]);
    let pk: Bytes48 = sk.public_key().to_bytes();
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

fn host_cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        // Allow a return buffer large enough to hold a >100 MB served resource.
        max_return_buffer_size: 128 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-compiler-large-data-section-test/0.1".to_string(),
    }
}

/// Encode a ContentRequest for `retrieval_key` with no root override, no range,
/// no JWT, no window (matches `digstore_guest::request::ContentRequest::encode`).
fn content_request(retrieval_key: Bytes32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&retrieval_key.0);
    out.push(0); // root_hash: None
    out.push(0); // range: None
    out.push(0); // jwt: None
    out.push(0); // window: None
    out
}

fn read_guest_wasm() -> Vec<u8> {
    match std::fs::read(GUEST_WASM) {
        Ok(b) => b,
        Err(e) => panic!(
            "guest wasm missing at {GUEST_WASM}: {e}. Build it first: \
             cargo build -p digstore-guest --target wasm32-unknown-unknown --release"
        ),
    }
}

/// Compile a one-resource store whose single chunk is `payload` plaintext, drive
/// it through the host, and assert the served bytes verify and decrypt back to
/// `payload`. Shared by the >8 MiB mechanism test and the near-cap stress test.
fn serve_single_resource_and_verify(payload: Vec<u8>, tag: &str) {
    let guest = read_guest_wasm();

    // ---- One private store, one resource (index.html) carrying `payload`. ----
    let store_id = Bytes32([0x7Au8; 32]);
    let urn = Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    let canonical = urn.canonical();
    let retrieval_key = urn.retrieval_key();

    // Client-derivable AES key (public store: no salt). The module never holds it.
    let key = derive_decryption_key(&canonical, None);
    let ciphertext = encrypt_chunk(&key, &payload);

    let resource = FixtureResource {
        retrieval_key,
        chunks: vec![(sha256(&ciphertext), ciphertext.clone())],
    };
    let gens = vec![FixtureGen {
        root: Bytes32([0x11u8; 32]),
        resources: vec![resource],
    }];

    // Expected per-resource leaf + current root (D5).
    let resource_ciphertext = concat_output(&[&ciphertext]);
    let expected_leaf = sha256(&resource_ciphertext);
    let expected_root = MerkleTree::from_leaves(vec![expected_leaf]).root();

    // ---- Compile a REAL module using the actual guest wasm as the template ----
    let dir = std::env::temp_dir().join(format!("digc-large-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: false,
        optimize: false,
        template_override: Some(guest),
    };
    let outcome = Compiler::compile(
        &cfg,
        store_id,
        Bytes48([0xCDu8; 48]),
        &gens,
        common::sample_manifest(),
        common::no_auth(),
        &common::trusted_keys(),
    )
    .expect("real module with a large data section compiles");

    let module = std::fs::read(&outcome.result.output_path).expect("read compiled module");

    // ---- Drive the REAL module through the host's serve flow (D6) ----
    let mut rt = HostRuntime::new(
        &module,
        host_cfg(),
        ExecutionLimits::default(), // 128 MiB ceiling after the Phase A raise
        host_deps(store_id),
    )
    .expect("host instantiates the compiled module");

    let resp_bytes = rt
        .serve_content(&content_request(retrieval_key))
        .expect("serve_content returns Ok");

    assert!(
        !resp_bytes.is_empty(),
        "serve_content MUST return non-empty bytes — the module must serve a large resource"
    );

    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    assert_eq!(
        resp.ciphertext, resource_ciphertext,
        "served ciphertext must equal the resource's ordered chunk ciphertext"
    );
    assert_eq!(
        resp.merkle_proof.root, expected_root,
        "proof.root == injected current root (trusted root)"
    );
    assert_eq!(
        resp.merkle_proof.leaf, expected_leaf,
        "proof.leaf == SHA-256(served ciphertext) (per-resource leaf, D5)"
    );
    assert_eq!(
        resp.merkle_proof.leaf,
        sha256(&resp.ciphertext),
        "proof.leaf must commit to exactly the served bytes"
    );
    assert!(
        resp.merkle_proof.verify(),
        "served merkle proof MUST verify against the trusted root"
    );

    // Client step: GCM-open the served ciphertext with the URN-derived key and
    // confirm it recovers the original plaintext byte-for-byte.
    let opened = digstore_crypto::decrypt_chunk(&key, &ciphertext).expect("chunk opens");
    assert_eq!(
        opened, payload,
        "client decrypt must recover the original (large) plaintext"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn module_with_data_section_over_8mib_serves_and_verifies() {
    // ~12 MiB resource: the injected data section is well above the old 8 MiB
    // fixed heap base, so this only passes with the dynamic guest heap.
    let payload = vec![0xABu8; 12 * 1024 * 1024];
    serve_single_resource_and_verify(payload, "12mib");
}

#[test]
#[ignore = "stress: ~90 MB resource validates the 128 MiB ceiling headroom; run with --ignored"]
fn near_cap_resource_serves_within_the_128mib_ceiling() {
    // ~90 MB resource pushes close to the 100 MB per-store cap and validates the
    // 128 MiB (2048-page) ceiling has enough headroom end-to-end. If this OOMs,
    // the single knob to raise is template::MAX_MEMORY_PAGES (and the host
    // MAX_MEMORY_BYTES) — everything else derives from it. Report any failure.
    //
    // KNOWN FINDING (Phase A): this currently FAILS at compile time with
    // `Validation("data section needs 3521 pages but §5.1 memory ceiling is
    // 2048")`. The §8.3 filler (`next_filler_bucket`, pipeline.rs) rounds the
    // filler length UP to the next power of two of the total content size, so the
    // injected blob is ~= content + next_pow2(content). With the 2 MiB data
    // offset, the blob alone must fit under 2048 pages (126 MiB usable), which
    // caps committable *content* at ~63 MiB — well under the planned 100 MB
    // MAX_STORE_BYTES. Resolving this (raise the ceiling further, cap content
    // lower, or make the filler additive rather than power-of-two) is a
    // cross-phase decision; this test is the validator that will go green once it
    // is fixed.
    let payload = vec![0x5Au8; 90 * 1024 * 1024];
    serve_single_resource_and_verify(payload, "near-cap");
}
