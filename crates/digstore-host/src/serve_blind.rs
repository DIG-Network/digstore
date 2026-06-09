//! Blind serve-by-retrieval-key helper (Artifact 3, `dighost`).
//!
//! `dighost` is the neutral DIG-Node pipe (paper §15, §18): it instantiates a
//! compiled module via [`HostRuntime`] and, on a request carrying a **32-byte
//! retrieval key**, calls [`HostRuntime::serve_content`] and streams the served
//! bytes (`ContentResponse` = ciphertext + merkle proof, or an indistinguishable
//! decoy) out. The host NEVER decrypts and never holds a URN — provider
//! blindness is structural. This module holds the wasmtime-only serve logic so
//! it can be unit/integration tested without pulling `object_store`/CLI deps
//! into the binary's storage layer.

use std::sync::Arc;

use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, MockChainSource, MockProver};

use crate::clock::FixedClock;
use crate::config::ExecutionLimits;
use crate::error::HostError;
use crate::runtime::{HostDeps, HostRuntime};

/// Fixed deterministic mock-chain block used by the blind serve path. The host
/// never consults a live chain to serve content; freshness for the embedded
/// attestation gate is satisfied by this deterministic block (matching the CLI
/// `serve` op and the `adv_self_serve` fixture).
fn mock_block() -> digstore_core::ChiaBlockRef {
    digstore_core::ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    }
}

/// Build the guest wire `ContentRequest` for a raw 32-byte retrieval key.
///
/// Framing (matches the CLI `serve::request_for`): `retrieval_key(32) ||
/// root_hash:None(0) || range:None(0) || jwt:None(0) || window:None(0)`. The
/// retrieval key is the ROOT-INDEPENDENT key the compiler stored at commit time
/// (the guest roots its proof at the injected `CurrentRoot`), so no root tag is
/// sent.
pub fn request_for_retrieval_key(retrieval_key: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    out.extend_from_slice(retrieval_key);
    out.push(0); // root_hash: None
    out.push(0); // range: None
    out.push(0); // jwt: None
    out.push(0); // window: None
    out
}

/// Inputs for the blind serve: the host's BLS identity (its public half must be
/// in the module's trusted set to serve real content), the store id, and a
/// deterministic clock timestamp.
pub struct BlindServeConfig {
    pub store_id: Bytes32,
    pub bls_secret: BlsSecretKey,
    pub bls_public: Bytes48,
    pub clock_unix: u64,
}

impl BlindServeConfig {
    /// Construct from a 32-byte host signing-key seed (the `signing_key.bin`
    /// written by `digstore init`). The public half is derived from the seed.
    pub fn from_seed(store_id: Bytes32, seed: &[u8]) -> Self {
        let sk = BlsSecretKey::from_seed(seed);
        let pk = sk.public_key().to_bytes();
        BlindServeConfig {
            store_id,
            bls_secret: sk,
            bls_public: pk,
            clock_unix: 1_700_000_000,
        }
    }
}

/// Build [`HostDeps`] for the blind serve path (MockProver + MockChainSource +
/// FixedClock), wiring the host's BLS identity in so attestation passes iff that
/// key is trusted by the module.
fn host_deps(cfg: BlindServeConfig) -> HostDeps {
    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = mock_block();
    let chain: Arc<dyn ChainSource> =
        Arc::new(MockChainSource::new(vec![block.clone()], cfg.clock_unix));
    let prover = MockProver::new(prover_sk, prover_pk, block);
    HostDeps {
        store_id: cfg.store_id,
        bls_secret: cfg.bls_secret,
        bls_public: cfg.bls_public,
        clock: Arc::new(FixedClock::new(cfg.clock_unix)),
        chain,
        prover: Arc::new(prover),
        // SECURITY: use real OS entropy, not a hardcoded seed. The host RNG seeds
        // attestation challenge nonces and the indistinguishable decoys returned
        // on a retrieval miss; a predictable seed would let an observer tell a
        // decoy from real content, defeating oblivious serving.
        // NOTE: the MockProver / MockChainSource / FixedClock above remain
        // placeholders and are NOT production-grade (forgeable proofs, fixed
        // freshness/time) — wiring the RISC0 backend and a real chain/clock is a
        // tracked follow-up.
        rng_seed: None,
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}

/// Instantiate the REAL compiled module from `module_bytes` and drive its own
/// serve flow for the given 32-byte retrieval key, returning the module's output
/// bytes EXACTLY as produced (§18.4: the host neither decrypts nor inspects the
/// payload). The returned bytes are a serialized `ContentResponse` envelope
/// (ciphertext + merkle proof) on a hit, or an indistinguishable non-verifying
/// decoy on a miss.
pub fn serve_blind(
    module_bytes: &[u8],
    retrieval_key: &[u8; 32],
    cfg: BlindServeConfig,
) -> Result<Vec<u8>, HostError> {
    let mut rt = HostRuntime::new(
        module_bytes,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(cfg),
    )?;
    let request = request_for_retrieval_key(retrieval_key);
    rt.serve_content(&request)
}
