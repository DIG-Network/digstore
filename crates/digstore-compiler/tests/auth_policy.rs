//! §4.1 / §5.2: "Per-store policy (such as JWT authentication) is likewise
//! carried as configuration and compiled into the module, not enforced by the
//! host." This proves the compiler embeds the CONFIGURED `AuthenticationInfo`
//! (not a hardcoded no-auth blob) and that a REAL compiled module reports it
//! back verbatim through the guest's `get_authentication_info` export.
//!
//! Two stores are compiled:
//!   * a JWT-required store -> guest reports requires_jwt=true with the
//!     configured jwks_url and accepted_algorithms;
//!   * a session-required store -> guest reports requires_session=true.

mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig};
use digstore_core::config::HostImportsConfig;
use digstore_core::{AuthenticationInfo, Bytes32, ChiaBlockRef, Decode, Decoder};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};
use std::sync::Arc;

const GUEST_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-unknown-unknown/release/digstore_guest.wasm"
);

fn host_deps(store_id: Bytes32) -> HostDeps {
    // The embedded trusted key is the public half of seed [42u8;32] (common.rs).
    let sk = BlsSecretKey::from_seed(&[42u8; 32]);
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

/// Compile a real module embedding `auth`, then read back the guest's
/// `get_authentication_info` and decode it as the canonical `AuthenticationInfo`.
fn compile_and_read_auth(tag: &str, auth: AuthenticationInfo) -> AuthenticationInfo {
    let guest = std::fs::read(GUEST_WASM).expect(
        "guest wasm must be built: \
         cargo build -p digstore-guest --target wasm32-unknown-unknown --release",
    );
    let dir = std::env::temp_dir().join(format!("digc-auth-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: false,
        optimize: false,
        template_override: Some(guest),
        // Small uniform budget keeps the emitted module tiny/fast.
        uniform_blob_len: 64 * 1024,
    };
    let gens = sample_generations();
    let outcome = Compiler::compile(
        &cfg,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        auth,
        &trusted_keys(),
    )
    .expect("real module compiles with the configured auth policy");
    let module = std::fs::read(&outcome.result.output_path).unwrap();

    let mut rt = HostRuntime::new(
        &module,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id()),
    )
    .expect("host instantiates the compiled module");

    let bytes = rt
        .get_authentication_info()
        .expect("get_authentication_info returns the embedded blob");
    let mut dec = Decoder::new(&bytes);
    let info = AuthenticationInfo::decode(&mut dec).expect("decodes as AuthenticationInfo");
    std::fs::remove_dir_all(&dir).ok();
    info
}

#[test]
fn jwt_required_store_reports_configured_jwt_policy() {
    let configured = AuthenticationInfo {
        requires_session: false,
        requires_jwt: true,
        jwks_url: Some("https://issuer.example/.well-known/jwks.json".to_string()),
        accepted_algorithms: vec!["RS256".to_string(), "ES256".to_string()],
    };
    let reported = compile_and_read_auth("jwt", configured.clone());
    assert_eq!(
        reported, configured,
        "guest must report the EXACT configured JWT auth policy (drift D-AUTH-01)"
    );
    assert!(reported.requires_jwt, "requires_jwt must be true");
    assert_eq!(
        reported.jwks_url.as_deref(),
        Some("https://issuer.example/.well-known/jwks.json")
    );
    assert_eq!(reported.accepted_algorithms, vec!["RS256", "ES256"]);
}

#[test]
fn session_required_store_reports_requires_session() {
    let configured = AuthenticationInfo {
        requires_session: true,
        requires_jwt: false,
        jwks_url: None,
        accepted_algorithms: vec![],
    };
    let reported = compile_and_read_auth("session", configured.clone());
    assert_eq!(
        reported, configured,
        "guest must report the EXACT configured session auth policy"
    );
    assert!(reported.requires_session, "requires_session must be true");
}
