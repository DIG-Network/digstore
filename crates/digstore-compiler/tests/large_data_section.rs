//! Phase A5 (uniform filler + 384 MiB ceiling) validator: a REAL compiled module
//! whose injected `DIGS` data section EXCEEDS the old 8 MiB fixed heap base must
//! still serve itself with a verifying merkle proof. This is the end-to-end proof
//! that the 6144-page (384 MiB) ceiling plus the guest's *dynamic* heap base
//! (computed above the data section) let stores up to the cap commit and serve,
//! and that the uniform-size filler makes every module the same size.
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
///
/// `uniform_blob_len` is the §8.3 uniform-size filler budget the module is padded
/// to. Returns the on-disk compiled `.wasm` size so callers can assert two stores
/// of very different content sizes compile to an IDENTICAL module size.
fn serve_single_resource_and_verify(payload: Vec<u8>, tag: &str, uniform_blob_len: usize) -> u64 {
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
        uniform_blob_len,
    };
    let outcome = Compiler::compile(
        &cfg,
        store_id,
        Bytes48([0xCDu8; 48]),
        &gens,
        common::sample_manifest(),
        common::no_auth(),
        &common::trusted_keys(),
        None,
    )
    .expect("real module with a large data section compiles");

    let module = std::fs::read(&outcome.result.output_path).expect("read compiled module");
    let module_size = outcome.result.output_size;

    // ---- Drive the REAL module through the host's serve flow (D6) ----
    let mut rt = HostRuntime::new(
        &module,
        host_cfg(),
        ExecutionLimits::default(), // 384 MiB ceiling after the A5 raise
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
    module_size
}

/// Compile the SAME one-resource store at a given uniform-blob budget and return
/// `(on_disk_module_size, embedded_current_root)`. The root is recomputed from
/// the module's embedded `MerkleNodes` and verified against the embedded
/// `CurrentRoot` by `verify_module_root` — i.e. exactly the content-derived root.
fn compile_and_extract_root(uniform_blob_len: usize, tag: &str) -> (u64, Bytes32) {
    let guest = read_guest_wasm();
    let store_id = digstore_core::sha256(&[0xCDu8; 48]); // SHA-256(pubkey) == store id
    let store_pubkey = Bytes48([0xCDu8; 48]);
    let urn = Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    let key = derive_decryption_key(&urn.canonical(), None);
    let ciphertext = encrypt_chunk(&key, b"fixed content for the root-invariance check");
    let resource = FixtureResource {
        retrieval_key: urn.retrieval_key(),
        chunks: vec![(sha256(&ciphertext), ciphertext)],
    };
    let gens = vec![FixtureGen {
        root: Bytes32([0x11u8; 32]),
        resources: vec![resource],
    }];

    let dir = std::env::temp_dir().join(format!("digc-rootinv-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: false,
        optimize: false,
        template_override: Some(guest),
        uniform_blob_len,
    };
    let outcome = Compiler::compile(
        &cfg,
        store_id,
        store_pubkey,
        &gens,
        common::sample_manifest(),
        common::no_auth(),
        &common::trusted_keys(),
        None,
    )
    .expect("compiles");
    let module = std::fs::read(&outcome.result.output_path).unwrap();
    let identity = digstore_compiler::verify_module_root(&module, &store_id)
        .expect("module self-identifies with a consistent embedded root");
    std::fs::remove_dir_all(&dir).ok();
    (outcome.result.output_size, identity.root)
}

#[test]
fn filler_length_does_not_change_the_merkle_root() {
    // The uniform filler (data section id 11) is unreferenced padding: changing
    // its length MUST NOT alter the resource leaves or the per-resource
    // `current_root`. Compile the SAME content at two very different budgets and
    // assert the embedded/recomputed root is IDENTICAL while module size differs.
    let (size_a, root_a) = compile_and_extract_root(64 * 1024, "small");
    let (size_b, root_b) = compile_and_extract_root(4 * 1024 * 1024, "big");
    assert_eq!(
        root_a, root_b,
        "filler length must not change the content-derived merkle root"
    );
    assert_ne!(
        size_a, size_b,
        "precondition: the two budgets produce different module sizes (filler differs)"
    );
}

#[test]
fn module_with_data_section_over_8mib_serves_and_verifies() {
    // ~12 MiB resource: the injected data section is well above the old 8 MiB
    // fixed heap base, so this only passes with the dynamic guest heap. A modest
    // uniform budget (16 MiB, comfortably above the ~12 MiB blob) keeps this fast.
    let payload = vec![0xABu8; 12 * 1024 * 1024];
    serve_single_resource_and_verify(payload, "12mib", 16 * 1024 * 1024);
}

#[test]
fn two_stores_of_different_sizes_compile_to_identical_module_size() {
    // §8.3 uniform-size filler: regardless of content size, every module is padded
    // to the same data-blob budget, so the on-disk module size is IDENTICAL. Use a
    // modest budget (16 MiB) so the test is fast but both blobs (~1 MiB vs ~8 MiB)
    // sit well below it and are padded up to the same total.
    let budget = 16 * 1024 * 1024;
    let small = serve_single_resource_and_verify(vec![0x11u8; 1024 * 1024], "uniform-sm", budget);
    let large =
        serve_single_resource_and_verify(vec![0x22u8; 8 * 1024 * 1024], "uniform-lg", budget);
    assert_eq!(
        small, large,
        "§8.3: two stores of very different content sizes MUST compile to the \
         same module size (uniform filler); got {small} vs {large}"
    );
}

#[test]
#[ignore = "stress: a full ~122 MiB resource compiled at the FULL 128 MiB production budget, served within the 384 MiB ceiling; run with --include-ignored"]
fn near_cap_store_serves_within_the_384mib_ceiling() {
    // A real store compiled at the FULL default 128 MiB uniform budget
    // (`digstore_compiler::FIXED_BLOB_LEN`) and served end-to-end through the
    // 6144-page (384 MiB) host ceiling. The module's heap base sits ABOVE the
    // ~128 MiB injected data region, so only ~254 MiB remains for the serve path.
    //
    // A6 (single-copy serve): the serve path no longer materializes the resource
    // multiple times. `get_content` builds the ciphertext exactly once
    // (`concat_output`, content.rs), the proof path STREAMS the output-commitment
    // hash (proof.rs — no resource-sized buffer), and `ContentResponse` is framed
    // by pre-sizing the wire buffer EXACTLY and moving the ciphertext in (abi.rs —
    // one copy, no Vec-doubling overshoot). Peak heap ≈ 128 MiB data region +
    // ciphertext + the (exact-sized) wire buffer, which fits a near-cap (~122 MiB)
    // resource inside the 384 MiB ceiling. A full ~122 MiB resource (≤
    // MAX_STORE_BYTES = 128 MB decimal) is the authoritative end-to-end validator.
    let payload = vec![0x5Au8; 122 * 1024 * 1024];
    serve_single_resource_and_verify(
        payload,
        "near-cap",
        digstore_compiler::FIXED_BLOB_LEN, // full 128 MiB production budget
    );
}
