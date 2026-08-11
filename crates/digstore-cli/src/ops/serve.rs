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
//!
//! ## §18.4 boundary: host returns verbatim; the CLI decode is client-side
//!
//! §18.4 says the host runtime "returns to the client exactly what the module
//! produced: it neither decrypts nor inspects the payload," and §18 says the
//! runtime "never parses content out of the module; it interacts only across the
//! ABI." `digstore-host` is faithful to that: `HostRuntime::serve_content`
//! returns the module's output bytes verbatim.
//!
//! This CLI is the `digstore` READER — the client on the trusting side of that
//! boundary (it holds the URN and the URN-derived keys, §11.3). [`serve_content_raw`]
//! surfaces the host's verbatim bytes; [`serve_content`] then DECODES the
//! [`ContentResponse`] envelope framing CLIENT-SIDE so the reader can run merkle
//! verification (§9.3) and AES-256-GCM decryption (§11). Decoding the envelope is
//! NOT decryption and NOT data-section inspection: the decoded
//! [`ContentResponse::ciphertext`] is still ciphertext. §18.4's "neither decrypts
//! nor inspects" constrains the host process proper, which remains faithful.

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

/// The REAL `digstore-guest` wasm, embedded at build time. Re-exported from the
/// shared stage→compile engine ([`digstore_stage`]), which now owns the SINGLE
/// embedded copy (its `build.rs` + `include_bytes!`) so the CLI and the
/// in-process node use the same wasm. `commit` compiles modules with this as the
/// compiler's `template_override` so the produced module is genuinely
/// self-serving through [`digstore_host::HostRuntime::serve_content`] (BINDING
/// contract D6).
pub use digstore_stage::embedded_guest_wasm;

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

/// Dependencies for an ANONYMOUS read runtime: no host identity at all.
///
/// Reading committed content consumes no identity, so this path carries none.
/// That is not a relaxed check but the absence of a subject to check: the guest's
/// content path builds its gate with `require_attestation: false`
/// (`digstore-guest/src/content.rs`), so nothing ever asks this host who it is.
/// Supplying a key here would be surface with no consumer; supplying a *stand-in*
/// key would be worse, because a store whose identity had been destroyed would
/// look healthy.
///
/// [`serve_proof`] is the sibling that genuinely does consume an identity, and it
/// loads one itself. See §13.6.
fn host_deps(store_id: Bytes32) -> HostDeps {
    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let chain = MockChainSource::new(vec![block.clone()], 1_700_000_000);
    let prover = MockProver::new(prover_sk, prover_pk, block);
    // ANONYMOUS: see this function's doc comment. `HostDeps::new` carries no
    // identity, and this path never calls `with_identity` — absence, not a
    // placeholder.
    //
    // The RNG is left at its default of real OS entropy rather than a constant
    // seed, converging on the convention `digstore_host::serve_blind` follows.
    //
    // SCOPE, stated honestly: this RNG backs `host_random_bytes`, whose only
    // live consumer is the guest's oblivious-access cover traffic. The §12
    // attestation nonce draw is unreachable here — `digstore-guest`'s content
    // path hardcodes `require_attestation: false` (dighub content is public
    // and must be servable by any node), so no module this CLI compiles takes
    // that branch. Nor is this closing a third-party attack: the party the
    // cover-traffic shuffle hides access patterns from is the HOST, and the
    // host is what supplies this randomness. The change removes a constant
    // seed that had no business outside a test fixture.
    HostDeps::new(
        store_id,
        Arc::new(FixedClock::new(1_700_000_000)),
        Arc::new(chain),
        Arc::new(prover),
        Bytes32([1u8; 32]),
    )
}

/// Instantiate the real host runtime over `module_path` (real wasmtime load /
/// validate / instantiate — this is how a corrupted CODE section surfaces).
///
/// The runtime is ANONYMOUS by construction: it reads no `signing_key.bin` and no
/// `trusted_keys.json`, so a store whose identity files are missing, unreadable,
/// or malformed still serves its committed content. Reading DIG content never
/// needs an account or a key (§5.3), and the two files are not inputs to it.
///
/// This deliberately reverses the earlier fail-closed load here. That load
/// misread the fix it belonged to: the real defect was SUBSTITUTING a stand-in
/// identity (an all-zero G1, a `from_seed(&[42u8; 32])` seed), and the cure for a
/// substituted key is to stop substituting, not to refuse the read. Refusing cost
/// availability on every read while buying nothing, because the guest never
/// consults the value — and it broke far more than `cat`: `checkout`, `dev`,
/// `deploy --preview` and `compute_status` reach this function too and have no
/// network ladder to fall through to.
fn instantiate_host(module_path: &Path, store_id: Bytes32) -> Result<HostRuntime, CliError> {
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    HostRuntime::new(
        &module_bytes,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id),
    )
    .map_err(|e| CliError::VerificationFailed(format!("module load/instantiate failed: {e:?}")))
}

/// Drive the REAL compiled module through [`HostRuntime::serve_content`] and
/// return the module's output bytes EXACTLY as the host runtime produced them
/// (BINDING contract D6).
///
/// This is the faithful §18.4 boundary: the host runtime "returns to the client
/// exactly what the module produced: it neither decrypts nor inspects the
/// payload." The returned bytes are the module's serialized [`ContentResponse`]
/// envelope (encoded + encrypted) — we hand them back VERBATIM and perform no
/// decode, no decrypt, and no data-section parsing here. The CLI's client-side
/// decode (and decryption) is a separate step in [`serve_content`].
pub fn serve_content_raw(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
) -> Result<Vec<u8>, CliError> {
    let store_id = urn.store_id;
    // No identity is loaded, and none is substituted for one — see
    // [`instantiate_host`]. `ctx` is still threaded through for the callers'
    // benefit, not for an identity read.
    let _ = ctx;
    let mut rt = instantiate_host(module_path, store_id)?;

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
    Ok(resp_bytes)
}

/// Serve content for `urn` by driving the REAL compiled module through
/// [`HostRuntime::serve_content`] (via [`serve_content_raw`]) and then DECODING
/// the returned [`ContentResponse`] envelope CLIENT-SIDE (BINDING contract D6).
///
/// §18.4 boundary: the host runtime returns the module's bytes verbatim
/// ([`serve_content_raw`]) — "it neither decrypts nor inspects the payload." This
/// function runs in the `digstore` reader (the client that holds the URN and the
/// keys, §11.3); decoding the envelope's framing is NOT decryption — the resulting
/// [`ContentResponse::ciphertext`] is still AES-256-GCM ciphertext that only the
/// caller's `client_crypto` step can open. The module serves itself: its
/// `get_content` performs the key-table lookup, oblivious gather, and builds a
/// per-resource merkle proof to the injected `CurrentRoot`. The CLI does NOT parse
/// the data section host-side. A retrieval miss returns a decoy whose proof does
/// not verify; the caller's `client_crypto` gate rejects it. The `root` argument
/// is the trusted root the caller verifies against (it is NOT trusted from the
/// module).
pub fn serve_content(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<ContentResponse, CliError> {
    let _ = root; // verification against the trusted root happens in client_crypto.

    // 1. Host runtime: raw module output, verbatim (no decrypt/inspect — §18.4).
    let resp_bytes = serve_content_raw(ctx, module_path, urn)?;

    // 2. Client-side decode of the envelope framing (NOT decryption). The
    //    decrypted plaintext is recovered later, in `client_crypto`, with the key.
    let mut dec = Decoder::new(&resp_bytes);
    let resp = ContentResponse::decode(&mut dec)
        .map_err(|e| CliError::VerificationFailed(format!("decode ContentResponse: {e:?}")))?;
    Ok(resp)
}

/// Serve and decrypt a committed resource, returning its plaintext bytes.
///
/// This is the shared serve+decrypt path used by both `cat` and `compute_status`.
/// It builds the canonical root-independent URN for `key`, drives the compiled
/// module through the host runtime via [`serve_content`], verifies the merkle
/// proof against `root`, and AES-256-GCM-opens the ciphertext using the store
/// salt — exactly the steps `commands/cat.rs` performs.
pub fn read_resource_plaintext(
    ctx: &crate::context::CliContext,
    cfg: &digstore_core::StoreConfig,
    root: &digstore_core::Bytes32,
    key: &str,
) -> anyhow::Result<Vec<u8>> {
    let urn = store_ops::canonical_resource_urn(cfg.store_id, key);
    let module_path = store_ops::module_path_for(ctx, &cfg.store_id, Some(*root))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let resp = serve_content(ctx, &module_path, &urn, *root).map_err(|e| anyhow::anyhow!("{e}"))?;
    let chunk_lens = store_ops::resource_chunk_lens(ctx, root, key).unwrap_or_default();
    let salt: Option<[u8; 32]> = match &cfg.visibility {
        digstore_core::Visibility::Private(s) => Some(s.0),
        digstore_core::Visibility::Public => None,
    };
    crate::ops::client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), root, &chunk_lens)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Serve a proof for `urn`. Produces a genuine `ExecutionProof` via the
/// `MockProver` over the served output commitment.
pub fn serve_proof(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<(ExecutionProof, Bytes32), CliError> {
    use digstore_prover::{build_public_input, MockVerifier, Prover, ServingInputs, Verifier};

    let resp = serve_content(ctx, module_path, urn, root)?;
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    // program_hash convention (deviation #3): SHA-256(template guest module bytes).
    // The module is compiled from the REAL guest wasm (D6), so the program hash is
    // over those embedded bytes.
    let program_hash = digstore_crypto::sha256(embedded_guest_wasm());

    // §13.7 "one key for both roles": the serving node signs the proof with the
    // SAME BLS key it uses for §12 host attestation — the key whose public half
    // the compiler embedded as the module's trusted host key. We load that
    // attestation signing key (init wrote `signing_key.bin`) rather than minting
    // an independent prover key, so node attribution is bound to the attestation
    // identity by construction.
    // FAIL CLOSED, and note that this is the OPPOSITE of the serve path above,
    // deliberately. Serving content consumes no identity, so it carries none;
    // signing a proof IS an act of attribution, so an absent or unreadable key
    // must stop it. Neither substituting a stand-in key (a proof attributed to a
    // world-known seed attributes serving work to nobody) nor proceeding without
    // one is available here — a proof needs a signer.
    //
    // Do not "simplify" this to match `instantiate_host`. The asymmetry is the
    // fix: identity became optional on the READ path only.
    let node_sk = store_ops::load_signing_key(ctx)?;
    let node_pk = node_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let prover = MockProver::new(node_sk, node_pk, block.clone());
    let public_input = build_public_input(&[0u8; digstore_prover::NONCE_LEN], &block);
    let serving = ServingInputs {
        retrieval_key: urn.retrieval_key(),
        roothash: root,
        chunk_ciphertext: vec![resp.ciphertext.clone()],
    };
    let proof = prover
        .prove(program_hash, &public_input, &serving)
        .map_err(|e| CliError::VerificationFailed(format!("prove: {e:?}")))?;

    // §13.7 + §12.2 (structural): the proof's node_pubkey MUST be a member of the
    // module's embedded §12 attestation trusted-key set, otherwise "one key for
    // both roles" is unenforced. Verify the binding against the persisted trusted
    // keys using the deterministic mock chain for freshness.
    // `unwrap_or_default()` is DELIBERATE here and is not the fallback-habit
    // defect the two loads above fix: an EMPTY trusted set is the strictest
    // possible set, not a permissive one. `verify_node_attested` rejects any
    // proof whose signer is absent from it (`NodeKeyNotAttested`,
    // `digstore-prover/src/prover.rs`), so an unreadable `trusted_keys.json`
    // makes the verification below fail rather than pass. Do not "fix" this into
    // a `?`; it would change nothing about safety.
    let trusted_node_keys = store_ops::load_trusted_keys(ctx)
        .map(|ks| {
            ks.into_iter()
                .map(|k| Bytes48(k.public_key))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let chain = digstore_prover::MockChainSource::new(vec![block.clone()], 1_700_000_000);
    MockVerifier
        .verify_node_attested(&proof, program_hash, &[root], &trusted_node_keys, &chain)
        .map_err(|e| CliError::VerificationFailed(format!("node-attested verify: {e:?}")))?;

    let _ = module_bytes;
    Ok((proof, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real committed store and return its context, root and module path
    /// — the fixture both call-site tests below need, because neither can be
    /// answered by inspecting a helper's return value.
    fn committed_store() -> (tempfile::TempDir, CliContext, Bytes32, std::path::PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        store_ops::init_store(&ctx, false, None, None, None, None, None, None).unwrap();

        let f = td.path().join("hello.txt");
        std::fs::write(&f, b"hello serve").unwrap();
        store_ops::add_path(&ctx, &f, Some("hello".into())).unwrap();
        let res = store_ops::commit(&ctx, None, empty_manifest()).unwrap();

        let store_id = ctx.find_store_id().unwrap();
        let module_path = store_ops::module_path_for(&ctx, &store_id, Some(res.roothash)).unwrap();
        (td, ctx, store_id, module_path)
    }

    /// FAIL CLOSED: the runtime `instantiate_host` ACTUALLY BUILDS must never pin
    /// the host RNG.
    ///
    /// Anchored at the call site on purpose. The obvious version of this test
    /// asserts `host_deps(..).rng_seed.is_none()`, which an attacker-equivalent
    /// refactor defeats trivially: inline a `HostDeps { rng_seed: Some(..), .. }`
    /// literal in `instantiate_host` and stop calling `host_deps` at all. The
    /// helper's contract stays intact, the production path is re-pinned, and the
    /// test stays green. So we instantiate through the real function and ask the
    /// runtime what it was built with — `HostRuntime::rng_is_deterministic`
    /// exists because the RNG is not observable through any export (the miss-path
    /// decoy is derived from the retrieval key, §14.2, so it is byte-stable
    /// whatever the RNG does).
    #[test]
    fn the_host_instantiate_host_builds_draws_real_entropy() {
        let (_td, _ctx, store_id, module_path) = committed_store();

        let rt =
            instantiate_host(&module_path, store_id).expect("an initialized store instantiates");

        assert!(
            !rt.rng_is_deterministic(),
            "the serve runtime must draw OS entropy; a pinned seed makes every \
             host_random_bytes draw reproducible from this source file"
        );
    }

    /// Delete every file that carries the store's identity.
    fn destroy_identity(ctx: &CliContext) {
        for f in ["signing_key.bin", "trusted_keys.json"] {
            let p = ctx.dig_dir.join(f);
            if p.exists() {
                std::fs::remove_file(&p).unwrap();
            }
        }
    }

    /// A store with NO identity still serves its committed content (#2712).
    ///
    /// Asserting `Ok` here would be a false green: the miss path returns a DECOY
    /// through the same `Ok` (§14.2), so a runtime that had silently stopped
    /// finding the resource would satisfy a bare success check. The assertion is
    /// therefore on the decrypted, merkle-verified PLAINTEXT — the one outcome a
    /// decoy cannot produce, because its proof does not verify against the
    /// trusted root.
    ///
    /// Fixture design: exactly one actor varies (the identity files), and the
    /// intact store is kept as a truthful control, so a store that had stopped
    /// serving for some unrelated reason could not read as a pass.
    #[test]
    fn a_store_with_no_identity_still_serves_committed_content() {
        let (_td, ctx, _store_id, _module_path) = committed_store();
        let cfg = ctx.load_config().unwrap();
        let root = store_ops::current_root(&ctx).unwrap().unwrap();

        // Control: the intact store returns the real bytes.
        let intact = read_resource_plaintext(&ctx, &cfg, &root, "hello")
            .expect("an intact store serves its own content");
        assert_eq!(intact, b"hello serve");

        destroy_identity(&ctx);

        let anonymous = read_resource_plaintext(&ctx, &cfg, &root, "hello").expect(
            "reading committed content consumes no identity, so a store whose \
             identity files are gone must still serve it",
        );
        assert_eq!(
            anonymous, intact,
            "an anonymous read must return the SAME real plaintext, not a decoy"
        );
    }

    /// The read runtime the PRODUCTION path builds carries no host identity.
    ///
    /// Anchored at the call site for the same reason as the RNG test above: an
    /// assertion on `host_deps(..)`'s return value is defeated by inlining a
    /// `.with_identity(..)` call into `instantiate_host`.
    /// The identity is not observable through any export on the content path, so
    /// `HostRuntime::has_host_public_key` exists to answer this.
    #[test]
    fn the_read_runtime_carries_no_host_identity() {
        let (_td, _ctx, store_id, module_path) = committed_store();

        let rt = instantiate_host(&module_path, store_id).expect("an initialized store serves");

        assert!(
            !rt.has_host_public_key(),
            "the read path must carry NO host identity; carrying one re-couples \
             every read to files it does not consume"
        );
    }

    /// The control that keeps the relaxation honest: identity became optional on
    /// the READ path ONLY.
    ///
    /// `serve_proof` signs, and signing is an act of attribution, so it must still
    /// refuse when the signing key is gone. Without this test the suite above is
    /// satisfied equally by "identity optional on reads" and by "identity optional
    /// everywhere" — the second being the security regression this change must not
    /// become. The message must also name the missing file, because a bare io
    /// error ("the system cannot find the file specified") fails just as closed
    /// and tells an operator nothing.
    #[test]
    fn serve_proof_still_refuses_without_a_signing_key() {
        let (_td, ctx, store_id, module_path) = committed_store();
        let urn = Urn {
            chain: "chia".into(),
            store_id,
            root_hash: None,
            resource_key: Some("hello".into()),
        };
        let root = store_ops::current_root(&ctx).unwrap().unwrap();

        // Control: with the identity intact, a proof is produced.
        serve_proof(&ctx, &module_path, &urn, root).expect("an intact store can sign a proof");

        destroy_identity(&ctx);

        let err = serve_proof(&ctx, &module_path, &urn, root)
            .expect_err("signing a proof REQUIRES an identity and must refuse without one");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("signing_key.bin"),
            "the refusal must name the missing identity file: {msg}"
        );
    }

    /// Revert-proof, two legs chained in ONE test because either alone is
    /// defeatable.
    ///
    /// `has_host_public_key` (the behavioural leg) is green both when the read
    /// path carries NOTHING and — crucially — it is NOT green when a stand-in key
    /// is substituted, so it catches a restored `unwrap_or(Bytes48([0u8; 48]))`.
    /// What it CANNOT catch is a re-added *refusal*: a runtime that never gets
    /// built because the read aborted earlier still trivially "carries no
    /// identity".
    ///
    /// SCOPE, stated exactly, because the previous wording overstated it: this
    /// leg catches a re-added refusal only in the `load_host_pubkey` form. It
    /// cannot catch the `load_signing_key` form — that token is deliberately
    /// ALLOWED below, since `serve_proof` lives in this same file and must keep
    /// calling it. The test that actually covers a refusal of any shape is the
    /// behavioural
    /// [`a_store_with_no_identity_still_serves_committed_content`], which deletes
    /// the identity files and demands real plaintext back; its control against
    /// over-relaxation is
    /// [`serve_proof_still_refuses_without_a_signing_key`].
    ///
    /// The banned tokens are the identity-carrying positions specifically, not
    /// seeds in general: `host_deps`'s MOCK PROVER key is a legitimate
    /// `from_seed(&[7u8; 32])` and has nothing to do with the host identity, so a
    /// blanket seed ban here would be a false failure that invites deletion.
    #[test]
    fn the_read_path_never_reaches_for_the_store_identity() {
        // ONE literal, used for both the count assertion and the split, so that
        // naming the marker here does not itself change the count.
        const TEST_MODULE_MARKER: &str = "#[cfg(test)]";

        let src = include_str!("serve.rs");

        // The split below assumes the FIRST marker opens the test module, so the
        // prefix it keeps is the whole production half. A new `#[cfg(test)]`
        // helper added ABOVE the read path would silently move that boundary and
        // narrow the guard to a prefix no longer containing the code it polices —
        // a guard that passes by scanning the wrong text. Fail loudly instead.
        let marker_count = src.matches(TEST_MODULE_MARKER).count();
        assert_eq!(
            marker_count, 2,
            "expected exactly 2 `{TEST_MODULE_MARKER}` occurrences in serve.rs \
             (the test-module attribute + this test's own marker literal), found \
             {marker_count}. A new one was added: re-check that the split below \
             still yields the WHOLE production half, then update this count."
        );

        // Cut the test module off so this file's own doc comments and fixtures do
        // not match themselves.
        let production = src
            .split(TEST_MODULE_MARKER)
            .next()
            .expect("this file has a test module");

        // `load_signing_key` is deliberately absent from this list: `serve_proof`
        // still calls it, and must.
        //
        // `identity: Some(` and `with_identity(` are the identity-carrying
        // positions under the `HostDeps { identity: Option<HostIdentity> }` shape.
        // They replaced `bls_public: Some(` / `bls_secret: Some(`, which those
        // fields no longer exist to produce — leaving those tokens here would be a
        // ban that can never fire, green by construction.
        for banned in [
            "load_host_pubkey",
            "identity: Some(",
            "with_identity(",
            "[0u8; 48]",
        ] {
            assert!(
                !production.contains(banned),
                "`{banned}` reappeared in this file's read path. Reading committed \
                 content consumes no identity: it must neither load one (which \
                 costs availability, #2712) nor substitute one (which fakes an \
                 identity nobody controls, #2553)."
            );
        }
    }
}
