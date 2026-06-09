//! BINDING contract D6: a REAL compiled module must serve itself.
//!
//! Compile a real `.wasm` from a tiny fixture (using the actual `digstore-guest`
//! wasm as the template), then drive it through `digstore_host::HostRuntime` and
//! assert `serve_content` returns NON-EMPTY bytes that decode to a
//! `ContentResponse` whose:
//!   * `merkle_proof.verify()` is true,
//!   * `merkle_proof.root == injected current_root`,
//!   * `merkle_proof.leaf == SHA-256(served ciphertext)` (per-resource leaf, D5),
//!   * GCM-decrypting each served chunk with the URN-derived key recovers the
//!     original plaintext (client step; the module never decrypts).
//!
//! This is the test that proves the compiler↔guest data-section drift is fixed:
//! the module is genuinely self-serving and its merkle proof genuinely verifies.

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
impl ResourceView for FixtureResource {
    fn resource_key(&self) -> Bytes32 {
        self.retrieval_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.chunks.clone()
    }
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
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-compiler-self-serving-test/0.1".to_string(),
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

#[test]
fn real_compiled_module_serves_itself_with_verifying_proof() {
    let guest = match std::fs::read(GUEST_WASM) {
        Ok(b) => b,
        Err(e) => panic!(
            "guest wasm missing at {GUEST_WASM}: {e}. Build it first: \
             cargo build -p digstore-guest --target wasm32-unknown-unknown --release"
        ),
    };

    // ---- Build a tiny fixture: one private store, one resource (index.html) ----
    let store_id = Bytes32([0x7Au8; 32]);
    let chain = "chia";
    let urn = Urn {
        chain: chain.to_string(),
        store_id,
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    let canonical = urn.canonical();
    let retrieval_key = urn.retrieval_key(); // SHA-256(canonical URN)

    // Client-derivable AES key (public store: no salt). The module never holds it.
    let key = derive_decryption_key(&canonical, None);

    // Plaintext content split into two chunks; each chunk is GCM-encrypted.
    let plain_a = b"<!doctype html><title>hello digstore</title>".to_vec();
    let plain_b = b"<p>the module serves itself</p>".to_vec();
    let ct_a = encrypt_chunk(&key, &plain_a);
    let ct_b = encrypt_chunk(&key, &plain_b);
    let original_plaintext = concat_output(&[&plain_a, &plain_b]);

    let resource = FixtureResource {
        retrieval_key,
        chunks: vec![(sha256(&ct_a), ct_a.clone()), (sha256(&ct_b), ct_b.clone())],
    };
    let store_root = Bytes32([0x11u8; 32]);
    let gens = vec![FixtureGen {
        root: store_root,
        resources: vec![resource],
    }];

    // Expected per-resource leaf + current root (D5).
    let resource_ciphertext = concat_output(&[&ct_a, &ct_b]);
    let expected_leaf = sha256(&resource_ciphertext);
    let expected_root = MerkleTree::from_leaves(vec![expected_leaf]).root();

    // ---- Compile a REAL module using the actual guest wasm as the template ----
    let dir = std::env::temp_dir().join(format!("digc-selfserve-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: false,
        optimize: false,
        template_override: Some(guest),
    };
    let store_pubkey = Bytes48([0xCDu8; 48]);
    let trusted = common::trusted_keys();
    let outcome = Compiler::compile(
        &cfg,
        store_id,
        store_pubkey,
        &gens,
        common::sample_manifest(),
        &trusted,
    )
    .expect("real module compiles");

    let module = std::fs::read(&outcome.result.output_path).expect("read compiled module");

    // ---- Drive the REAL module through the host's serve flow (D6) ----
    let mut rt = HostRuntime::new(
        &module,
        host_cfg(),
        ExecutionLimits::default(),
        host_deps(store_id),
    )
    .expect("host instantiates the compiled module");

    let req = content_request(retrieval_key);
    let resp_bytes = rt.serve_content(&req).expect("serve_content returns Ok");

    assert!(
        !resp_bytes.is_empty(),
        "serve_content MUST return non-empty bytes — the module must serve itself"
    );

    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    // The served ciphertext must be the resource's ordered chunk ciphertext.
    assert_eq!(
        resp.ciphertext, resource_ciphertext,
        "served ciphertext must equal the resource's ordered chunk ciphertext"
    );

    // Merkle proof: genuinely verifies, roots match, leaf = SHA-256(ciphertext).
    assert_eq!(
        resp.roothash, expected_root,
        "response root == injected current root"
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

    // Client step: GCM-open each chunk with the URN-derived key (the module never
    // decrypts; only a client holding the URN can). Reassemble == original.
    let opened_a = digstore_crypto::decrypt_chunk(&key, &ct_a).expect("chunk A opens");
    let opened_b = digstore_crypto::decrypt_chunk(&key, &ct_b).expect("chunk B opens");
    let reassembled = concat_output(&[&opened_a, &opened_b]);
    assert_eq!(
        reassembled, original_plaintext,
        "client decrypt+reassemble must recover the original plaintext"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_compiled_module_miss_returns_decoy_failing_the_client_proof_gate() {
    let guest = std::fs::read(GUEST_WASM)
        .expect("guest wasm must be built (cargo build -p digstore-guest --target wasm32-unknown-unknown --release)");

    let store_id = Bytes32([0x7Au8; 32]);
    let urn = Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    let canonical = urn.canonical();
    let key = derive_decryption_key(&canonical, None);
    let ct_a = encrypt_chunk(&key, b"only resource chunk A");
    let ct_b = encrypt_chunk(&key, b"only resource chunk B");
    let real_ciphertext = concat_output(&[&ct_a, &ct_b]);

    let resource = FixtureResource {
        retrieval_key: urn.retrieval_key(),
        chunks: vec![(sha256(&ct_a), ct_a.clone()), (sha256(&ct_b), ct_b.clone())],
    };
    let gens = vec![FixtureGen {
        root: Bytes32([0x11u8; 32]),
        resources: vec![resource],
    }];
    let expected_root = MerkleTree::from_leaves(vec![sha256(&real_ciphertext)]).root();

    let dir = std::env::temp_dir().join(format!("digc-selfmiss-{}", std::process::id()));
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
        &common::trusted_keys(),
    )
    .expect("compiles");
    let module = std::fs::read(&outcome.result.output_path).unwrap();

    let mut rt = HostRuntime::new(
        &module,
        host_cfg(),
        ExecutionLimits::default(),
        host_deps(store_id),
    )
    .unwrap();

    // A request for a resource that does NOT exist -> decoy. The decoy is
    // wire-indistinguishable from a real hit (same root field, real-looking
    // ciphertext shape) but its merkle proof is structurally real yet
    // UNVERIFIABLE, so the client's `proof.verify()` integrity gate rejects it.
    let bogus = Bytes32([0xEEu8; 32]);
    let resp_bytes = rt.serve_content(&content_request(bogus)).expect("serve ok");
    assert!(
        !resp_bytes.is_empty(),
        "decoy is still a non-empty response"
    );
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes as ContentResponse");

    assert_ne!(
        resp.ciphertext, real_ciphertext,
        "a miss must NOT return the real resource ciphertext"
    );
    assert!(
        !resp.merkle_proof.verify(),
        "the decoy's proof must FAIL the client verify() gate"
    );

    // Sanity: the real resource still serves and verifies on this same module.
    let real = rt
        .serve_content(&content_request(urn.retrieval_key()))
        .expect("serve real ok");
    let mut dec = Decoder::new(&real);
    let real_resp = ContentResponse::decode(&mut dec).expect("decodes");
    assert_eq!(real_resp.ciphertext, real_ciphertext);
    assert_eq!(real_resp.merkle_proof.root, expected_root);
    assert!(real_resp.merkle_proof.verify(), "real hit must verify");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn obfuscated_real_module_still_serves_itself_with_verifying_proof() {
    // §17.1: obfuscation is behavior-preserving. An OBFUSCATED real module must
    // still serve itself, with the merkle proof verifying against the same root.
    let guest = std::fs::read(GUEST_WASM)
        .expect("guest wasm must be built (cargo build -p digstore-guest --target wasm32-unknown-unknown --release)");

    let store_id = Bytes32([0x7Au8; 32]);
    let urn = Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    let key = derive_decryption_key(&urn.canonical(), None);
    let ct = encrypt_chunk(&key, b"obfuscated yet self-serving");
    let real_ciphertext = concat_output(&[&ct]);
    let expected_root = MerkleTree::from_leaves(vec![sha256(&real_ciphertext)]).root();

    let resource = FixtureResource {
        retrieval_key: urn.retrieval_key(),
        chunks: vec![(sha256(&ct), ct.clone())],
    };
    let gens = vec![FixtureGen {
        root: Bytes32([0x11u8; 32]),
        resources: vec![resource],
    }];

    let dir = std::env::temp_dir().join(format!("digc-selfobf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: true, // <-- exercise the obfuscation pass on a real module
        optimize: false,
        template_override: Some(guest),
    };
    let outcome = Compiler::compile(
        &cfg,
        store_id,
        Bytes48([0xCDu8; 48]),
        &gens,
        common::sample_manifest(),
        &common::trusted_keys(),
    )
    .expect("obfuscated module compiles");
    let module = std::fs::read(&outcome.result.output_path).unwrap();

    let mut rt = HostRuntime::new(
        &module,
        host_cfg(),
        ExecutionLimits::default(),
        host_deps(store_id),
    )
    .unwrap();
    let resp_bytes = rt
        .serve_content(&content_request(urn.retrieval_key()))
        .expect("obfuscated module serves");
    assert!(
        !resp_bytes.is_empty(),
        "obfuscated module must still serve itself"
    );
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decodes");
    assert_eq!(resp.ciphertext, real_ciphertext);
    assert_eq!(resp.merkle_proof.root, expected_root);
    assert_eq!(resp.merkle_proof.leaf, sha256(&resp.ciphertext));
    assert!(
        resp.merkle_proof.verify(),
        "obfuscation must preserve a verifying proof (§17.1)"
    );

    std::fs::remove_dir_all(&dir).ok();
}
