//! Content path (§7,8,14). Gate (attestation/session/JWT/temporal) -> key-table
//! lookup -> oblivious gather -> ContentResponse, else a decoy. The guest never
//! decrypts; it returns ciphertext + a merkle proof to the generation root.

use digstore_core::merkle::MerkleTree;
use digstore_core::MerkleProof;

/// Emit an inclusion proof for `leaf_index` using the same rules as core:
/// leaf = SHA-256(chunk), node = SHA-256(left||right), odd node carried up,
/// root = generation root. Delegates to the core tree's proof builder so guest
/// and client agree byte-for-byte. Out-of-range indices yield an empty proof
/// rooted at the tree root (which fails `verify`, as expected).
pub fn emit_merkle_proof(tree: &MerkleTree, leaf_index: usize) -> MerkleProof {
    tree.prove(leaf_index).unwrap_or_else(|| MerkleProof {
        leaf: tree.root(),
        path: alloc::vec::Vec::new(),
        root: tree.root(),
    })
}

use crate::datasection::{DataSection, SectionId};
use crate::decoy::decoy_content_response;
use crate::host::DigHost;
use crate::oblivious::build_access_plan;
use crate::request::ContentRequest;
use crate::temporal::within_window;
use alloc::vec::Vec;
use digstore_core::codec::Decode;
use digstore_core::serving::concat_output;
use digstore_core::{Bytes32, ContentResponse};

pub struct GateConfig {
    pub require_attestation: bool,
    pub require_jwt: bool,
    pub expected_iss: Option<alloc::string::String>,
    pub expected_aud: Option<alloc::string::String>,
}

impl GateConfig {
    /// Build the gate config the wasm ABI uses.
    ///
    /// Host attestation is DISABLED: dighub content is public and must be servable
    /// by ANY node, so the guest does NOT gate real content on the serving host's
    /// BLS key being in an embedded trusted set. Tying serving to a per-node trusted
    /// key also forced each node to re-key the module — which rewrites the `.dig`
    /// and so the program hash (the execution-proof identity must be identical on
    /// every node). Dropping the gate keeps a single, stable program hash network-
    /// wide AND lets anonymous nodes serve. The privacy property is UNAFFECTED: the
    /// oblivious decoy-on-miss path (an absent retrieval key still returns an
    /// indistinguishable non-verifying decoy) is independent of attestation.
    ///
    /// The JWT gate is unchanged — it is driven by the store's embedded AuthInfo
    /// (`requires_jwt`, §6.2), so a store that opts into JWT auth still enforces the
    /// claims + JWKS signature (§6.3).
    pub fn from_embedded(ds: &DataSection) -> GateConfig {
        let require_jwt = embedded_auth_info(ds)
            .map(|i| i.requires_jwt)
            .unwrap_or(false);
        GateConfig {
            require_attestation: false,
            require_jwt,
            expected_iss: None,
            expected_aud: None,
        }
    }
}

pub enum ContentOutcome {
    Real(ContentResponse),
    Decoy(ContentResponse),
}

/// Read chunk ciphertext at `index` from the ChunkPool section
/// (count u32 BE, then per chunk: len u32 BE || bytes).
fn read_chunk(ds: &DataSection, index: u32) -> Option<Vec<u8>> {
    let pool = ds.section(SectionId::ChunkPool)?;
    if pool.len() < 4 {
        return None;
    }
    let count = u32::from_be_bytes([pool[0], pool[1], pool[2], pool[3]]);
    if index >= count {
        return None;
    }
    let mut p = 4usize;
    for i in 0..count {
        if p + 4 > pool.len() {
            return None;
        }
        let len = u32::from_be_bytes([pool[p], pool[p + 1], pool[p + 2], pool[p + 3]]) as usize;
        p += 4;
        if p + len > pool.len() {
            return None;
        }
        if i == index {
            return Some(pool[p..p + len].to_vec());
        }
        p += len;
    }
    None
}

/// Decode the embedded TrustedKeys section (id 5) into a [`TrustedSet`] of
/// 48-byte BLS G1 public keys. The body matches the compiler's codec
/// (`data_section::encode_trusted_keys`): u32 BE count, then per entry a raw
/// `[u8; 48]` public key followed by a `String` label (u32 BE len + bytes).
/// An absent or malformed section yields an empty set (the gate then fails
/// closed, matching §12.3's "refuse a module whose trusted set is empty").
fn embedded_trusted_set(ds: &DataSection) -> crate::attestation::TrustedSet {
    use digstore_core::codec::Decoder;

    let body = match ds.section(SectionId::TrustedKeys) {
        Some(b) => b,
        None => return crate::attestation::TrustedSet::from_pubkeys(&[]),
    };
    let mut dec = Decoder::new(body);
    let count = match u32::decode(&mut dec) {
        Ok(c) => c,
        Err(_) => return crate::attestation::TrustedSet::from_pubkeys(&[]),
    };
    let mut keys: Vec<[u8; 48]> = Vec::new();
    for _ in 0..count {
        let pk = match <[u8; 48]>::decode(&mut dec) {
            Ok(p) => p,
            Err(_) => break,
        };
        // Skip the label string (length-prefixed) — only the key matters here.
        if alloc::string::String::decode(&mut dec).is_err() {
            break;
        }
        keys.push(pk);
    }
    crate::attestation::TrustedSet::from_pubkeys(&keys)
}

/// Run the gate chain. Returns Err with a decoy-trigger reason if any gate fails.
fn gate<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ContentRequest,
    cfg: &GateConfig,
) -> Result<(), ()> {
    // Obfuscation seam: a default-true opaque predicate the compiler pass targets.
    if !crate::obfuscation_hooks::opaque_true() {
        return Err(());
    }
    // Temporal first (cheapest).
    if !within_window(&req.window, host.current_time()) {
        return Err(());
    }
    // Attestation gate (§12.2): issue a fresh challenge, have the host sign it,
    // then VERIFY the returned signature against the embedded trusted set. Any
    // failure (untrusted key / bad signature / stale / malformed / absent
    // trusted set) fails closed -> the caller returns a decoy.
    if cfg.require_attestation {
        let nonce = host.random_bytes(32).map_err(|_| ())?;
        if nonce.len() < 32 {
            return Err(());
        }
        let mut nonce32 = [0u8; 32];
        nonce32.copy_from_slice(&nonce[..32]);

        // The signed message is the challenge: nonce(32) || store_id(32) || time(u64 BE).
        // `signed_time` is the timestamp BOUND INTO the challenge — the value the
        // host commits to via its signature (§12.1, "unix seconds, for freshness").
        let signed_time = host.current_time();
        let challenge = crate::attestation::build_challenge(nonce32, ds.store_id().0, signed_time);

        // A real host returns a signed AttestationResponse; an error => fail closed.
        let resp_bytes = host.create_attestation(&challenge).map_err(|_| ())?;
        let resp = digstore_core::AttestationResponse::from_bytes(&resp_bytes).map_err(|_| ())?;

        // §12.2: verify the BLS signature over the challenge under
        // host_public_key, check the challenge timestamp for FRESHNESS against the
        // module's own clock read at verification time, and check trusted-set
        // membership. Reading the clock again here (rather than reusing
        // `signed_time`) makes the freshness check meaningful: a response bound to
        // a stale timestamp — e.g. a slow or replaying host — fails closed.
        let now = host.current_time();
        let trusted = embedded_trusted_set(ds);
        crate::attestation::verify_attestation(
            &trusted,
            &challenge,
            &resp.host_public_key,
            &resp.signature,
            signed_time,
            now,
        )
        .map_err(|_| ())?;
    }
    // JWT gate (§6.3, §12.4). When a JWT gate is configured, the guest enforces
    // BOTH the claim checks AND the token's cryptographic signature against the
    // store's trusted JWKS. Any failure fails closed -> the caller returns a
    // Decoy (never real content, never a 404).
    if cfg.require_jwt {
        // §12.4: "The session is the precondition for any JWT-authorization logic
        // the module chooses to enforce before releasing real content." Require an
        // active session BEFORE running any JWT logic; with no/invalid session
        // (the host returns NoSession/SessionExpired, i.e. verify_session()==false)
        // fail closed -> Decoy, even if the presented token would itself validate.
        if !host.verify_session() {
            return Err(());
        }
        let jwt = req.jwt.as_ref().ok_or(())?;
        let policy = crate::jwt::ClaimPolicy {
            now: host.current_time(),
            expected_iss: cfg.expected_iss.as_deref(),
            expected_aud: cfg.expected_aud.as_deref(),
        };
        // The JWKS endpoint is advertised by the store's embedded AuthInfo
        // section (§6.2 get_authentication_info); the host fetches it over the
        // session-gated `jwks_fetch` import. A missing JWKS URL / fetch failure /
        // unverifiable signature all fail closed.
        let jwks_url = embedded_jwks_url(ds).ok_or(())?;
        if verify_request_jwt(host, jwks_url.as_bytes(), jwt, &policy).is_err() {
            return Err(());
        }
    }
    Ok(())
}

/// Outcome of the gate + lookup + oblivious gather, before the served bytes are
/// assembled. Lets the `ContentResponse` (struct) and the single-copy wire framer
/// share one gather pass while assembling the served ciphertext only once each.
enum GatheredOutcome {
    /// A hit: the per-slot gathered chunks (cover + real), the indices of the real
    /// chunks within `gathered` (in C9 order), the inclusion proof, and the root.
    Real {
        gathered: Vec<Vec<u8>>,
        real_positions: Vec<usize>,
        merkle_proof: MerkleProof,
        root: Bytes32,
    },
    /// A miss / gate failure: a deterministic success-shaped decoy response.
    Decoy(ContentResponse),
}

/// Run gate -> key lookup -> oblivious gather. Shared by [`serve_content`] (which
/// materializes a `ContentResponse`) and [`serve_content_wire`] (which frames the
/// wire response in a single pre-sized buffer). The gather reads EVERY slot in the
/// plan (cover + real) so the access pattern is uniform.
fn gather_content<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ContentRequest,
    cfg: &GateConfig,
) -> GatheredOutcome {
    let root = req.root_hash.unwrap_or_else(|| ds.current_root());
    if gate(host, ds, req, cfg).is_err() {
        return GatheredOutcome::Decoy(decoy_content_response(&req.retrieval_key, &root));
    }
    let entry = match ds.lookup_key(&req.retrieval_key) {
        Some(e) => e,
        None => return GatheredOutcome::Decoy(decoy_content_response(&req.retrieval_key, &root)),
    };

    // Oblivious gather: pool size from ChunkPool count.
    let pool = ds.section(SectionId::ChunkPool).unwrap_or(&[]);
    let pool_size = if pool.len() >= 4 {
        u32::from_be_bytes([pool[0], pool[1], pool[2], pool[3]])
    } else {
        0
    };
    let plan = build_access_plan(&entry.chunk_indices, pool_size, |c| {
        host.random_bytes(c)
            .unwrap_or_else(|_| alloc::vec![0u8; c as usize])
    });

    // Read EVERY slot in the plan (cover + real) so the access pattern is uniform,
    // then keep only the real chunks in original order.
    let mut gathered: Vec<Vec<u8>> = Vec::with_capacity(plan.order.len());
    for idx in &plan.order {
        gathered.push(read_chunk(ds, *idx).unwrap_or_default());
    }
    let merkle_proof = build_real_proof(ds, &entry, &root);
    GatheredOutcome::Real {
        gathered,
        real_positions: plan.real_positions,
        merkle_proof,
        root,
    }
}

/// Build a real ContentResponse for a hit: oblivious gather of the real chunk
/// indices (with cover reads + shuffle), concatenate real ciphertext in order,
/// attach a merkle proof to the current root.
pub fn serve_content<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ContentRequest,
    cfg: &GateConfig,
) -> ContentOutcome {
    match gather_content(host, ds, req, cfg) {
        GatheredOutcome::Decoy(resp) => ContentOutcome::Decoy(resp),
        GatheredOutcome::Real {
            gathered,
            real_positions,
            merkle_proof,
            root,
        } => {
            // CONVENTIONS C9: assemble output with the shared `concat_output`
            // ordering so it matches the prover's `ServingInputs::output_bytes`.
            let real_slices: Vec<&[u8]> = real_positions
                .iter()
                .map(|pos| gathered[*pos].as_slice())
                .collect();
            // Per-chunk CIPHERTEXT lengths (in C9 order), so a streaming client can split the
            // plain-concatenated ciphertext and GCM-SIV-open each chunk. Single-chunk resources
            // carry a one-element vec; the client treats that identically to "whole blob".
            let chunk_lens: Vec<u32> = real_slices.iter().map(|s| s.len() as u32).collect();
            let ciphertext = concat_output(&real_slices);
            ContentOutcome::Real(ContentResponse {
                ciphertext,
                merkle_proof,
                roothash: root,
                chunk_lens,
            })
        }
    }
}

/// Single-copy serve (§7 wire framing, byte-identical to
/// `ContentResponse::encode`): produce the framed wire response —
/// `Vec<u8>` ciphertext (u32 BE len + bytes) || merkle_proof || roothash — in a
/// single, EXACTLY pre-sized buffer.
///
/// The served ciphertext can be ~122 MiB at the store cap and the guest's bump
/// allocator never frees, so this assembles the real chunk ciphertext directly
/// into the final wire buffer (no separate `concat_output` intermediate, no
/// `ContentResponse.ciphertext` copy, no Vec growth-doubling overshoot). The only
/// resource-sized live allocations are the per-slot gathered chunks (the oblivious
/// gather, left intact) and this one wire buffer. The bytes a client decodes are
/// unchanged — this is purely an allocation/copy optimization.
pub fn serve_content_wire<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ContentRequest,
    cfg: &GateConfig,
) -> Vec<u8> {
    use digstore_core::codec::{Encode, Encoder};

    match gather_content(host, ds, req, cfg) {
        // Decoys are tiny; the naive encode is fine.
        GatheredOutcome::Decoy(resp) => {
            let mut enc = Encoder::new();
            resp.encode(&mut enc);
            enc.finish()
        }
        GatheredOutcome::Real {
            gathered,
            real_positions,
            merkle_proof,
            root,
        } => {
            // Frame the small tail (merkle proof + roothash + chunk_lens) on its own first; a
            // proof is at most a few hundred bytes and chunk_lens is 4 bytes/chunk — never
            // resource-sized. The tail MUST be byte-identical to `ContentResponse::encode`'s
            // suffix (merkle_proof || roothash || chunk_lens) so a decoded response matches.
            let mut tail = Encoder::new();
            merkle_proof.encode(&mut tail);
            root.encode(&mut tail);
            let chunk_lens: Vec<u32> = real_positions
                .iter()
                .map(|p| gathered[*p].len() as u32)
                .collect();
            chunk_lens.encode(&mut tail);
            let tail = tail.finish();

            // Served ciphertext length = sum of the real chunk lengths in C9 order
            // (== `concat_output(&real_slices).len()`).
            let cipher_len: usize = real_positions.iter().map(|p| gathered[*p].len()).sum();

            // ONE allocation for the entire wire response; no overshoot.
            let mut enc = Encoder::with_capacity(4 + cipher_len + tail.len());
            (cipher_len as u32).encode(&mut enc); // Vec<u8> length prefix (4 BE)
            for pos in &real_positions {
                enc.write_bytes(gathered[*pos].as_slice()); // single copy into the reserved buffer
            }
            enc.write_bytes(&tail);
            enc.finish()
        }
    }
}

/// Build a genuinely-verifying inclusion proof (contract D5).
///
/// Rebuild `MerkleTree::from_leaves(decode_merkle_leaves(MerkleNodes))`, find the
/// served resource's leaf index = its position among the resources sorted in
/// ascending `static_key` order (`Bytes32` has no `Ord`, so we compare the raw
/// 32-byte arrays lexicographically), and emit `tree.prove(index)`. The returned
/// `MerkleProof { leaf, path, root }` satisfies `verify()` and its `root` equals
/// the injected `CurrentRoot` (== `tree.root()`).
///
/// If the `MerkleNodes` section is absent or malformed (unit fixtures without an
/// injected tree), fall back to a single-leaf tree over the served resource so
/// callers still get a self-consistent, verifying proof rooted at `root`.
fn build_real_proof(
    ds: &DataSection,
    entry: &digstore_core::KeyTableEntry,
    root: &Bytes32,
) -> MerkleProof {
    use digstore_core::datasection::decode_merkle_leaves;

    let leaves = ds
        .section(SectionId::MerkleNodes)
        .and_then(|body| decode_merkle_leaves(body).ok());

    match leaves {
        Some(leaves) if !leaves.is_empty() => {
            let leaf_index = resource_leaf_index(ds, &entry.static_key);
            let tree = MerkleTree::from_leaves(leaves);
            tree.prove(leaf_index).unwrap_or_else(|| MerkleProof {
                leaf: tree.root(),
                path: alloc::vec::Vec::new(),
                root: tree.root(),
            })
        }
        // No injected merkle tree: single-leaf tree over the served resource, so
        // the proof is self-consistent and verifies against its own root.
        _ => {
            let leaf = *root;
            MerkleProof {
                leaf,
                path: alloc::vec::Vec::new(),
                root: *root,
            }
        }
    }
}

/// Leaf index of the served resource = the number of KeyTable entries whose
/// `static_key` sorts strictly before the served key (ascending by raw 32 bytes).
/// The KeyTable order is the leaf order (D3/D5), so this rank addresses the
/// correct leaf even if the table is not pre-sorted.
fn resource_leaf_index(ds: &DataSection, served_key: &Bytes32) -> usize {
    let body = match ds.section(SectionId::KeyTable) {
        Some(b) => b,
        None => return 0,
    };
    let mut dec = digstore_core::codec::Decoder::new(body);
    let count = match u32::decode(&mut dec) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut rank = 0usize;
    for _ in 0..count {
        match digstore_core::KeyTableEntry::decode(&mut dec) {
            Ok(e) => {
                if e.static_key.0 < served_key.0 {
                    rank += 1;
                }
            }
            Err(_) => break,
        }
    }
    rank
}

/// Decode the embedded AuthInfo section (§6.2 get_authentication_info) into the
/// canonical [`digstore_core::AuthenticationInfo`]. Returns `None` if the section
/// is absent or malformed (the gate then treats JWT as not-required / no JWKS).
fn embedded_auth_info(ds: &DataSection) -> Option<digstore_core::AuthenticationInfo> {
    use digstore_core::codec::Decoder;
    let body = ds.section(SectionId::AuthInfo)?;
    let mut dec = Decoder::new(body);
    digstore_core::AuthenticationInfo::decode(&mut dec).ok()
}

/// Read the store's advertised JWKS endpoint from the embedded AuthInfo section.
/// Returns `None` if absent/malformed or it carries no `jwks_url`, in which case
/// a JWT gate fails closed (no key source -> Decoy).
fn embedded_jwks_url(ds: &DataSection) -> Option<alloc::string::String> {
    embedded_auth_info(ds).and_then(|i| i.jwks_url)
}

/// Decode, claim-check, AND cryptographically verify a request JWT (§6.3).
///
/// 1. Decode the three base64url segments and enforce the temporal/issuer/
///    audience claims (`exp`/`nbf`/`iss`/`aud`).
/// 2. Fetch the store's JWKS over the session-gated `jwks_fetch` host import
///    (the gate has already confirmed an active session), parse it, and select
///    the verifying key by the token's `kid` (or the sole key if no `kid`).
/// 3. Verify the signature over `header_b64.payload_b64` with the matching
///    algorithm (RS256 via `rsa` PKCS#1 v1.5 over SHA-256, ES256 via `p256`).
///
/// Any failure — bad claims, no/unparseable JWKS, unknown `kid`, unsupported
/// `alg`, or a signature that does not verify — returns `Err`. The caller turns
/// that into a Decoy, so a token whose claims look valid but whose signature is
/// absent/tampered/from the wrong key still fails closed.
pub fn verify_request_jwt<H: DigHost + ?Sized>(
    host: &H,
    jwks_url: &[u8],
    jwt: &[u8],
    policy: &crate::jwt::ClaimPolicy,
) -> Result<(), crate::jwt::JwtError> {
    let parts = crate::jwt::decode_unverified(jwt)?;
    crate::jwt::check_claims(&parts.claims, policy)?;

    // Fetch + parse the trusted JWKS (session-gated at the host boundary, and we
    // re-check the session here so the guest also fails closed on a buggy host).
    let jwks_bytes = crate::session::gated_jwks_fetch(host, jwks_url)
        .map_err(|_| crate::jwt::JwtError::UnknownKey)?;
    let jwks = crate::jwt::parse_jwks(&jwks_bytes)?;

    // Select the key by kid; if the token has no kid, fall back to the single key
    // when the JWKS contains exactly one (a JWKS with no usable key -> UnknownKey).
    let jwk = match parts.kid.as_deref() {
        Some(kid) => jwks
            .iter()
            .find(|k| k.kid == kid)
            .ok_or(crate::jwt::JwtError::UnknownKey)?,
        None => match jwks.as_slice() {
            [only] => only,
            _ => return Err(crate::jwt::JwtError::UnknownKey),
        },
    };

    crate::jwt::verify_signature(&parts.alg, jwk, &parts.signing_input, &parts.signature)
}
