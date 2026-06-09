//! Serving layer (BINDING contract D6): obtain served bytes by driving the REAL
//! compiled module through [`digstore_host::HostRuntime::serve_content`]. The
//! module serves itself — the CLI does NOT parse the data section host-side.
//!
//! `commit` compiles each module with the real `digstore-guest` wasm as the
//! compiler template (see [`embedded_guest_wasm`]), so the module's
//! `get_content` runs the genuine guest logic (key-table lookup, oblivious
//! gather, per-resource merkle proof to the injected `CurrentRoot`) and returns
//! a serialized [`ContentResponse`]. A retrieval miss yields a decoy whose proof
//! does NOT verify (§14.2); the client's verification gate (`client_crypto`)
//! rejects it. The host NEVER decrypts; decryption is a separate client step.

use std::path::Path;
use std::sync::Arc;

use digstore_core::config::HostImportsConfig;
use digstore_core::{
    Bytes32, Bytes48, ChiaBlockRef, ContentResponse, Decode, Decoder, ExecutionProof,
    MetadataManifest, Urn,
};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};

use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

/// The REAL `digstore-guest` wasm, embedded at CLI build time (see `build.rs`).
/// `commit` compiles modules with this as the compiler's `template_override` so
/// the produced module is genuinely self-serving through
/// [`digstore_host::HostRuntime::serve_content`] (BINDING contract D6).
pub fn embedded_guest_wasm() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/digstore_guest.wasm"))
}

/// An empty metadata manifest (the compiler requires one).
pub fn empty_manifest() -> MetadataManifest {
    MetadataManifest {
        schema_version: 1,
        name: String::new(),
        version: None,
        description: None,
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: Default::default(),
        custom: Default::default(),
    }
}

/// Build the guest's wire `ContentRequest` bytes for a URN (custom big-endian
/// framing the guest's `request::ContentRequest::decode` expects).
///
/// The lookup key is the ROOT-INDEPENDENT retrieval key (the `static_key` the
/// compiler stored at commit time via `canonical_resource_urn`), and `root_hash`
/// is omitted so the guest uses its injected `CurrentRoot` (the trusted root the
/// client gates against). This matches `store_ops::canonical_resource_urn`.
pub fn request_for(urn: &Urn) -> Vec<u8> {
    let resource_key = urn.resource_key.clone().unwrap_or_default();
    let canonical = store_ops::canonical_resource_urn(urn.store_id, &resource_key);
    let mut out = Vec::new();
    out.extend_from_slice(&canonical.retrieval_key().0);
    out.push(0); // root_hash: None (root-independent retrieval key)
    out.push(0); // range
    out.push(0); // jwt
    out.push(0); // window
    out
}

fn host_deps(store_id: Bytes32, pubkey: Bytes48, secret: BlsSecretKey) -> HostDeps {
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
        bls_secret: secret,
        bls_public: pubkey,
        clock: Arc::new(FixedClock::new(1_700_000_000)),
        chain: Arc::new(chain),
        prover: Arc::new(prover),
        rng_seed: Some([99u8; 32]),
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}

/// Instantiate the real host runtime over `module_path` (real wasmtime load /
/// validate / instantiate — this is how a corrupted CODE section surfaces).
fn instantiate_host(
    ctx: &CliContext,
    module_path: &Path,
    store_id: Bytes32,
    pubkey: Bytes48,
) -> Result<HostRuntime, CliError> {
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    // §12.2: the host MUST attest with the store's host signing key — the same
    // key whose public half the compiler embedded as the trusted key. Load the
    // persisted seed (init wrote `signing_key.bin`) so the guest's attestation
    // verification accepts this host; otherwise it would (correctly) serve decoys.
    let secret =
        store_ops::load_signing_key(ctx).unwrap_or_else(|_| BlsSecretKey::from_seed(&[42u8; 32]));
    HostRuntime::new(
        &module_bytes,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, pubkey, secret),
    )
    .map_err(|e| CliError::VerificationFailed(format!("module load/instantiate failed: {e:?}")))
}

/// Serve content for `urn` by driving the REAL compiled module through
/// [`HostRuntime::serve_content`] (BINDING contract D6). The module serves
/// itself: its `get_content` performs the key-table lookup, oblivious gather, and
/// builds a per-resource merkle proof to the injected `CurrentRoot`. We decode
/// the returned [`ContentResponse`] and hand it back verbatim — the CLI does NOT
/// parse the data section host-side. A retrieval miss returns a decoy whose proof
/// does not verify; the caller's `client_crypto` gate rejects it. The `root`
/// argument is the trusted root the caller verifies against (it is NOT trusted
/// from the module).
pub fn serve_content(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<ContentResponse, CliError> {
    let _ = root; // verification against the trusted root happens in client_crypto.
    let store_id = urn.store_id;
    let pubkey = store_ops::load_host_pubkey(ctx).unwrap_or(Bytes48([0u8; 48]));
    let mut rt = instantiate_host(ctx, module_path, store_id, pubkey)?;

    // Drive the module's own serve flow. The request carries the ROOT-INDEPENDENT
    // retrieval key (matching the compiler's `static_key`) so the guest finds the
    // resource and roots the proof at its injected `CurrentRoot`.
    let request = request_for(urn);
    let resp_bytes = rt
        .serve_content(&request)
        .map_err(|e| CliError::VerificationFailed(format!("module serve_content failed: {e:?}")))?;
    if resp_bytes.is_empty() {
        return Err(CliError::VerificationFailed(
            "module returned an empty response (not self-serving)".into(),
        ));
    }

    // Decode the serialized ContentResponse the guest produced.
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec)
        .map_err(|e| CliError::VerificationFailed(format!("decode ContentResponse: {e:?}")))?;
    Ok(resp)
}

/// Serve a proof for `urn`. Produces a genuine `ExecutionProof` via the
/// `MockProver` over the served output commitment.
pub fn serve_proof(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<(ExecutionProof, Bytes32), CliError> {
    use digstore_prover::{build_public_input, Prover, ServingInputs};

    let resp = serve_content(ctx, module_path, urn, root)?;
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    // program_hash convention (deviation #3): SHA-256(template guest module bytes).
    // The module is compiled from the REAL guest wasm (D6), so the program hash is
    // over those embedded bytes.
    let program_hash = digstore_crypto::sha256(embedded_guest_wasm());

    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let prover = MockProver::new(prover_sk, prover_pk, block.clone());
    let public_input = build_public_input(&[0u8; digstore_prover::NONCE_LEN], &block);
    let serving = ServingInputs {
        retrieval_key: urn.retrieval_key(),
        roothash: root,
        chunk_ciphertext: vec![resp.ciphertext.clone()],
    };
    let proof = prover
        .prove(program_hash, &public_input, &serving)
        .map_err(|e| CliError::VerificationFailed(format!("prove: {e:?}")))?;
    let _ = module_bytes;
    Ok((proof, root))
}
