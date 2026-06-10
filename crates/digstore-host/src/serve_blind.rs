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
//!
//! # Proof backend / chain / clock selection (residual #3)
//!
//! The prover, chain source, and clock are **injectable** via
//! [`BlindServeDeps`]. The default ([`BlindServeDeps::mock`]) is the
//! deterministic mock/fixed trio (`MockProver` + `MockChainSource` +
//! `FixedClock`) so the default build and the existing tests stay green
//! WITHOUT the RISC0 toolchain.
//!
//! A caller that wants production trust can swap in a real chain
//! ([`digstore_prover::CoinsetChainSource`] against coinset.org) and a real
//! wall clock ([`crate::clock::SystemClock`]) with no extra dependencies, and
//! — **behind the `risc0` cargo feature** — a real `Risc0Prover` via
//! [`BlindServeDeps::with_risc0_prover`]. The `risc0` feature pulls the
//! `risc0-build` step (`embed_methods`) which compiles the zkVM guest ELF and
//! therefore requires the RISC0 toolchain (`r0vm`/`rzup`); it is NOT enabled in
//! the default build/CI here. Producing real execution proofs is "wiring done;
//! flip the feature + install the toolchain" — see `SECURITY.md` residual #3.

use std::sync::Arc;

use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, MockChainSource, MockProver, Prover};

use crate::clock::{Clock, FixedClock};
use crate::config::ExecutionLimits;
use crate::error::HostError;
use crate::runtime::{HostDeps, HostRuntime};

/// Fixed deterministic mock-chain block used by the default blind serve path.
/// The host never consults a live chain to serve content with the mock trio;
/// freshness for the embedded attestation gate is satisfied by this
/// deterministic block (matching the CLI `serve` op and the `adv_self_serve`
/// fixture).
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
/// in the module's trusted set to serve real content) and the store id.
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

/// Injectable proof backend, chain source, and clock for the blind serve path
/// (residual #3). The prover, chain, and clock are supplied separately so a
/// caller can mix a real chain/clock with a mock prover (default build) or a
/// real `Risc0Prover` (the `risc0` feature, which needs the toolchain).
///
/// Default ([`BlindServeDeps::mock`]): `MockProver` + `MockChainSource` +
/// `FixedClock`, all pinned to a deterministic block — keeps existing tests and
/// the toolchain-free default build green.
pub struct BlindServeDeps {
    /// Execution-proof backend. Default mock is forgeable (deviation #3); a real
    /// `Risc0Prover` requires the `risc0` feature + the RISC0 toolchain.
    pub prover: Arc<dyn Prover>,
    /// Source of Chia chain state for attestation freshness (§13.8). Default is
    /// `MockChainSource`; [`digstore_prover::CoinsetChainSource`] is the real one.
    pub chain: Arc<dyn ChainSource>,
    /// Wall-clock source (§12). Default is a `FixedClock`; `SystemClock` is real.
    pub clock: Arc<dyn Clock>,
}

impl BlindServeDeps {
    /// The default deterministic trio: `MockProver` + `MockChainSource` +
    /// `FixedClock`, all bound to [`mock_block`] / `clock_unix`. This is what the
    /// toolchain-free default build and the existing serve tests use.
    ///
    /// NOTE: the MockProver / MockChainSource / FixedClock are placeholders and
    /// are NOT production-grade (forgeable proofs, fixed freshness/time). Swap in
    /// [`Self::with_real_chain_clock`] (+ the `risc0` feature for a real prover)
    /// for production trust.
    pub fn mock(clock_unix: u64) -> Self {
        let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
        let prover_pk = prover_sk.public_key();
        let block = mock_block();
        let chain: Arc<dyn ChainSource> =
            Arc::new(MockChainSource::new(vec![block.clone()], clock_unix));
        let prover = MockProver::new(prover_sk, prover_pk, block);
        BlindServeDeps {
            prover: Arc::new(prover),
            chain,
            clock: Arc::new(FixedClock::new(clock_unix)),
        }
    }

    /// Replace the chain source with a real one (e.g.
    /// [`digstore_prover::CoinsetChainSource`]) and the clock with
    /// [`crate::clock::SystemClock`]. The prover is left as-is (still the mock
    /// unless also overridden via [`Self::with_prover`] / a `risc0` build), so
    /// this alone does NOT make proofs unforgeable — it makes freshness and time
    /// real.
    pub fn with_real_chain_clock(mut self, chain: Arc<dyn ChainSource>) -> Self {
        self.chain = chain;
        self.clock = Arc::new(crate::clock::SystemClock);
        self
    }

    /// Override just the proof backend.
    pub fn with_prover(mut self, prover: Arc<dyn Prover>) -> Self {
        self.prover = prover;
        self
    }

    /// Build a REAL `Risc0Prover` bound to the chain's current peak block and a
    /// freshly-generated node BLS identity, and install it as the proof backend.
    ///
    /// Available ONLY under the `risc0` cargo feature, which pulls
    /// `risc0-build` (`embed_methods`) — that step compiles the zkVM guest ELF
    /// and therefore REQUIRES the RISC0 toolchain (`r0vm`/`rzup`). It is not
    /// built in the default build/CI here. `node_seed` is the 32-byte seed for
    /// the node's proof-signing BLS key.
    ///
    /// Returns an error if the chain peak cannot be fetched.
    #[cfg(feature = "risc0")]
    pub fn with_risc0_prover(mut self, node_seed: &[u8]) -> Result<Self, HostError> {
        use digstore_prover::risc0_backend::Risc0Prover;
        let peak = self
            .chain
            .get_peak()
            .map_err(|e| HostError::Wasmtime(format!("chain get_peak for risc0 prover: {e}")))?;
        let node_sk = digstore_crypto::bls::SecretKey::from_seed(node_seed);
        let node_pk = node_sk.public_key();
        self.prover = Arc::new(Risc0Prover::new(node_sk, node_pk, peak));
        Ok(self)
    }
}

/// Build [`HostDeps`] for the blind serve path from the store identity in `cfg`
/// and the injected proof backend / chain / clock in `deps`, wiring the host's
/// BLS identity in so attestation passes iff that key is trusted by the module.
fn host_deps(cfg: BlindServeConfig, deps: BlindServeDeps) -> HostDeps {
    HostDeps {
        store_id: cfg.store_id,
        bls_secret: cfg.bls_secret,
        bls_public: cfg.bls_public,
        clock: deps.clock,
        chain: deps.chain,
        prover: deps.prover,
        // SECURITY: use real OS entropy, not a hardcoded seed. The host RNG seeds
        // attestation challenge nonces and the indistinguishable decoys returned
        // on a retrieval miss; a predictable seed would let an observer tell a
        // decoy from real content, defeating oblivious serving.
        rng_seed: None,
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}

/// Instantiate the REAL compiled module from `module_bytes` and drive its own
/// serve flow for the given 32-byte retrieval key with the DEFAULT mock/fixed
/// trio, returning the module's output bytes EXACTLY as produced (§18.4: the
/// host neither decrypts nor inspects the payload). The returned bytes are a
/// serialized `ContentResponse` envelope (ciphertext + merkle proof) on a hit,
/// or an indistinguishable non-verifying decoy on a miss.
///
/// To supply a real chain/clock (and, under the `risc0` feature, a real prover)
/// use [`serve_blind_with`].
pub fn serve_blind(
    module_bytes: &[u8],
    retrieval_key: &[u8; 32],
    cfg: BlindServeConfig,
) -> Result<Vec<u8>, HostError> {
    let deps = BlindServeDeps::mock(cfg.clock_unix);
    serve_blind_with(module_bytes, retrieval_key, cfg, deps)
}

/// Like [`serve_blind`] but with a caller-supplied [`BlindServeDeps`] (prover,
/// chain source, clock). This is the injection point for residual #3: a real
/// `CoinsetChainSource` + `SystemClock` (+ a `risc0` `Risc0Prover`) can be
/// passed in here while the default `serve_blind` keeps the mock trio.
pub fn serve_blind_with(
    module_bytes: &[u8],
    retrieval_key: &[u8; 32],
    cfg: BlindServeConfig,
    deps: BlindServeDeps,
) -> Result<Vec<u8>, HostError> {
    let mut rt = HostRuntime::new(
        module_bytes,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(cfg, deps),
    )?;
    let request = request_for_retrieval_key(retrieval_key);
    rt.serve_content(&request)
}
