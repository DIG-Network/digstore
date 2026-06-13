//! # dig-client (read-crypto WASM)
//!
//! Browser read-crypto for dighub. The visitor's browser does ALL the crypto so
//! dighub / the CDN stay blind (Frontend Decision Q6; API §17). This module
//! exposes, as `globalThis.digClient` (and as an ES module), the three read-path
//! primitives the frontend needs to VIEW content WITHOUT trusting dighub:
//!
//! 1. **URN reconstruction** for a `(store_id, root, resource_key)` and the
//!    `retrieval_key = SHA-256(canonical_urn)` (Digstore §6.1/§7.3; API §17).
//! 2. **AES-256-GCM-SIV decryption** of a resource's ciphertext under the
//!    URN-derived key (Digstore §11.1/§11.2; RFC 8452).
//! 3. **Inclusion-proof verification** of served ciphertext against a root the
//!    client trusts FROM THE CHAIN — never from the serving response (Digstore
//!    §9; API §17/§18). A decoy's proof cannot verify against the real root.
//!
//! The crypto is byte-identical to the host-side `digstore-crypto` (see
//! `crypto.rs`) and reuses `digstore-core`'s canonical `Urn` and `MerkleProof`.
//!
//! ## Trust model
//! The trusted root is read by the caller directly from coinset.org (the
//! singleton's anchored root), passed in here, and the merkle proof must chain to
//! it. The serving CDN is untrusted: a tampered chunk fails the leaf check; a
//! decoy / wrong-store response fails because its proof does not chain to the
//! chain-anchored root; a wrong/missing private salt yields a wrong key whose
//! GCM-SIV tag fails. Confidentiality rests on URN secrecy and (private stores)
//! the secret salt — never on decoys (API §17 honest note).

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use base64::Engine;
use digstore_core::codec::Decode;
use digstore_core::crypto::{decrypt_chunk, derive_decryption_key};
use digstore_core::{
    resource_leaf, Bytes32, MerkleProof, SecretSalt, Urn, CHAIN, DEFAULT_RESOURCE_KEY,
};
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build the canonical ROOT-INDEPENDENT resource URN. Both the retrieval key and
/// the AES key derive from this exact form (root dropped), matching the
/// host/CLI commit-time derivation (`canonical_resource_urn`): a URN whose key is
/// stable across roots so the retrieval key and decryption key never change when
/// a new generation is committed.
fn canonical_resource_urn(store_id: Bytes32, resource_key: &str) -> Urn {
    Urn {
        chain: CHAIN.to_string(),
        store_id,
        root_hash: None,
        resource_key: Some(resource_key.to_string()),
    }
}

/// Parse a 64-hex store id, mapping any error to a JS error.
fn parse_store_id(store_id_hex: &str) -> Result<Bytes32, JsError> {
    Bytes32::from_hex(store_id_hex.trim())
        .map_err(|_| JsError::new("store_id must be 64 lowercase hex characters"))
}

/// Parse a 32-byte secret salt from optional hex (private stores). `None`/empty
/// means a public store (URN alone decrypts).
fn parse_salt(salt_hex: Option<String>) -> Result<Option<[u8; 32]>, JsError> {
    match salt_hex {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => {
            let b = Bytes32::from_hex(s.trim())
                .map_err(|_| JsError::new("secret salt must be 64 lowercase hex characters"))?;
            Ok(Some(b.0))
        }
    }
}

/// Decode a base64 merkle proof (the `X-Dig-Inclusion-Proof` header /
/// `merkle_proof_b64` envelope field) into a `MerkleProof`. The wire encoding is
/// the Chia big-endian streamable codec (`MerkleProof::to_bytes`).
fn decode_proof_b64(proof_b64: &str) -> Result<MerkleProof, JsError> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(proof_b64.trim().as_bytes())
        .map_err(|_| JsError::new("inclusion proof is not valid base64"))?;
    MerkleProof::from_bytes(&raw)
        .map_err(|_| JsError::new("inclusion proof is not a valid merkle proof encoding"))
}

/// The verification core (no JS types): the served `ciphertext` must be the
/// proof's leaf (`leaf = SHA-256(ciphertext)`), the path must fold to
/// `proof.root`, and `proof.root` must equal `trusted_root`. This is the exact
/// gate the CLI applies in `client_crypto::verify_chunk_inclusion` (Digstore
/// §9.3): leaf-binding, then merkle fold, then chain-anchored root equality.
fn verify_inclusion_core(
    ciphertext: &[u8],
    proof: &MerkleProof,
    trusted_root: &Bytes32,
) -> Result<(), &'static str> {
    let computed_leaf = resource_leaf(ciphertext);
    if computed_leaf != proof.leaf {
        return Err("content does not match proof leaf (tampered ciphertext)");
    }
    if !proof.verify() {
        return Err("merkle path does not resolve to the declared root");
    }
    if &proof.root != trusted_root {
        return Err("merkle root does not match the chain-anchored trusted root");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// (1) URN reconstruction
// ---------------------------------------------------------------------------

/// Reconstruct the canonical ROOT-INDEPENDENT resource URN string for a store +
/// resource key: `urn:dig:chia:<store_id>[/<resource_key>]`. An empty resource
/// key resolves to the §8.5 default view `index.html`. This is the form whose
/// SHA-256 is the retrieval key and whose bytes seed the AES key.
#[wasm_bindgen(js_name = reconstructUrn)]
pub fn reconstruct_urn(store_id_hex: &str, resource_key: &str) -> Result<String, JsError> {
    let store_id = parse_store_id(store_id_hex)?;
    let key = if resource_key.is_empty() {
        DEFAULT_RESOURCE_KEY
    } else {
        resource_key
    };
    Ok(canonical_resource_urn(store_id, key).canonical())
}

/// Reconstruct a ROOT-PINNED display URN: `urn:dig:chia:<store_id>:<root>/<key>`.
/// Useful for sharing a URN bound to a specific generation; the retrieval/AES
/// keys still use the rootless form (`reconstructUrn`).
#[wasm_bindgen(js_name = reconstructUrnWithRoot)]
pub fn reconstruct_urn_with_root(
    store_id_hex: &str,
    root_hex: &str,
    resource_key: &str,
) -> Result<String, JsError> {
    let store_id = parse_store_id(store_id_hex)?;
    let root = Bytes32::from_hex(root_hex.trim())
        .map_err(|_| JsError::new("root must be 64 lowercase hex characters"))?;
    let key = if resource_key.is_empty() {
        DEFAULT_RESOURCE_KEY
    } else {
        resource_key
    };
    let urn = Urn {
        chain: CHAIN.to_string(),
        store_id,
        root_hash: Some(root),
        resource_key: Some(key.to_string()),
    };
    Ok(urn.canonical())
}

/// `retrieval_key = SHA-256(canonical_rootless_urn)`, lowercase hex (Digstore
/// §7.3; API §17). The CDN is addressed by this hash; the URN itself is never
/// sent. An empty resource key resolves to `index.html`.
#[wasm_bindgen(js_name = retrievalKey)]
pub fn retrieval_key(store_id_hex: &str, resource_key: &str) -> Result<String, JsError> {
    let store_id = parse_store_id(store_id_hex)?;
    let key = if resource_key.is_empty() {
        DEFAULT_RESOURCE_KEY
    } else {
        resource_key
    };
    Ok(canonical_resource_urn(store_id, key)
        .retrieval_key()
        .to_hex())
}

// ---------------------------------------------------------------------------
// (2) Key derivation + AES-256-GCM-SIV decryption
// ---------------------------------------------------------------------------

/// Derive the 32-byte AES-256 content key for a resource (Digstore §11.1/§11.4),
/// returned as lowercase hex. `salt_hex` is the 32-byte private-store secret salt
/// (omit / pass `null` for public stores). Mixing in a wrong/missing salt yields
/// a wrong key whose GCM-SIV tag will not verify.
#[wasm_bindgen(js_name = deriveKey)]
pub fn derive_key(
    store_id_hex: &str,
    resource_key: &str,
    salt_hex: Option<String>,
) -> Result<String, JsError> {
    let store_id = parse_store_id(store_id_hex)?;
    let salt = parse_salt(salt_hex)?;
    let key = if resource_key.is_empty() {
        DEFAULT_RESOURCE_KEY
    } else {
        resource_key
    };
    let canonical = canonical_resource_urn(store_id, key).canonical();
    let derived = derive_decryption_key(&canonical, salt.map(SecretSalt).as_ref());
    Ok(hex::encode(derived))
}

/// Decrypt a SINGLE GCM-SIV chunk under an explicit 32-byte `key` (hex). Returns
/// the plaintext bytes. A failed tag check (tamper / wrong key) is an error.
/// Low-level escape hatch; most callers want `decryptResource`.
#[wasm_bindgen(js_name = decryptChunk)]
pub fn decrypt_chunk_js(key_hex: &str, ciphertext: &[u8]) -> Result<Vec<u8>, JsError> {
    let key = Bytes32::from_hex(key_hex.trim())
        .map_err(|_| JsError::new("key must be 64 lowercase hex characters"))?;
    decrypt_chunk(&key.0, ciphertext).map_err(|_| {
        JsError::new("AES-256-GCM-SIV tag verification failed (wrong key or tampered ciphertext)")
    })
}

/// Full read pipeline for a resource's served ciphertext (Digstore §9.3 + §11),
/// returning the decrypted plaintext bytes. Steps, in order (gate-then-decrypt):
///
/// 1. **Integrity gate** — verify the served bytes' merkle inclusion against the
///    chain-anchored `trusted_root_hex` (proof base64 from `X-Dig-Inclusion-Proof`).
/// 2. **Confidentiality** — derive the URN key, split the PLAIN-concatenated
///    chunk ciphertexts by `chunk_lens` (the per-chunk CIPHERTEXT byte lengths in
///    order; D5/C9 — NO length framing on the wire), and AES-256-GCM-SIV-open
///    each, concatenating plaintext in order.
///
/// `chunk_lens` may be empty for the common single-chunk resource (the whole blob
/// is one GCM-SIV ciphertext). They MUST sum to `ciphertext.len()`.
#[wasm_bindgen(js_name = decryptResource)]
#[allow(clippy::too_many_arguments)]
pub fn decrypt_resource(
    store_id_hex: &str,
    resource_key: &str,
    ciphertext: &[u8],
    proof_b64: &str,
    trusted_root_hex: &str,
    salt_hex: Option<String>,
    chunk_lens: Option<Vec<u32>>,
) -> Result<Vec<u8>, JsError> {
    let store_id = parse_store_id(store_id_hex)?;
    let salt = parse_salt(salt_hex)?;
    let trusted_root = Bytes32::from_hex(trusted_root_hex.trim())
        .map_err(|_| JsError::new("trusted root must be 64 lowercase hex characters"))?;
    let proof = decode_proof_b64(proof_b64)?;

    // 1) integrity: the served bytes are committed under the chain-anchored root.
    verify_inclusion_core(ciphertext, &proof, &trusted_root).map_err(JsError::new)?;

    // 2) confidentiality: derive the key, split the plain concat, open each chunk.
    let key = if resource_key.is_empty() {
        DEFAULT_RESOURCE_KEY
    } else {
        resource_key
    };
    let canonical = canonical_resource_urn(store_id, key).canonical();
    let aes_key = derive_decryption_key(&canonical, salt.map(SecretSalt).as_ref());

    let plan: Vec<usize> = match chunk_lens {
        Some(lens) if !lens.is_empty() => lens.into_iter().map(|l| l as usize).collect(),
        _ => alloc::vec![ciphertext.len()],
    };
    let total: usize = plan.iter().sum();
    if total != ciphertext.len() {
        return Err(JsError::new(&format!(
            "served ciphertext length {} does not match expected chunk total {}",
            ciphertext.len(),
            total
        )));
    }

    let mut plaintext = Vec::with_capacity(ciphertext.len());
    let mut p = 0usize;
    for len in plan {
        let ct = &ciphertext[p..p + len];
        p += len;
        let pt = decrypt_chunk(&aes_key, ct).map_err(|_| {
            JsError::new(
                "AES-256-GCM-SIV tag verification failed (wrong key/salt or tampered ciphertext)",
            )
        })?;
        plaintext.extend_from_slice(&pt);
    }
    Ok(plaintext)
}

/// Convenience wrapper around [`decrypt_resource`] returning the plaintext as a
/// UTF-8 string (for HTML/text resources rendered into the sandbox iframe).
#[wasm_bindgen(js_name = decryptResourceToText)]
pub fn decrypt_resource_to_text(
    store_id_hex: &str,
    resource_key: &str,
    ciphertext: &[u8],
    proof_b64: &str,
    trusted_root_hex: &str,
    salt_hex: Option<String>,
    chunk_lens: Option<Vec<u32>>,
) -> Result<String, JsError> {
    let bytes = decrypt_resource(
        store_id_hex,
        resource_key,
        ciphertext,
        proof_b64,
        trusted_root_hex,
        salt_hex,
        chunk_lens,
    )?;
    String::from_utf8(bytes).map_err(|_| JsError::new("decrypted resource is not valid UTF-8 text"))
}

// ---------------------------------------------------------------------------
// (3) Inclusion-proof verification
// ---------------------------------------------------------------------------

/// Verify that `ciphertext` is included under `trusted_root_hex` via the base64
/// merkle `proof_b64` (Digstore §9.3; API §18). Returns `true` on success and
/// `false` on ANY verification failure (tampered bytes, non-chaining path, or a
/// root mismatch / decoy) — a decoy or wrong-store response returns `false`
/// rather than throwing, so a caller can treat it as "not found in this store".
/// Throws only on malformed inputs (bad base64 / hex / proof encoding).
#[wasm_bindgen(js_name = verifyInclusion)]
pub fn verify_inclusion(
    ciphertext: &[u8],
    proof_b64: &str,
    trusted_root_hex: &str,
) -> Result<bool, JsError> {
    let trusted_root = Bytes32::from_hex(trusted_root_hex.trim())
        .map_err(|_| JsError::new("trusted root must be 64 lowercase hex characters"))?;
    let proof = decode_proof_b64(proof_b64)?;
    Ok(verify_inclusion_core(ciphertext, &proof, &trusted_root).is_ok())
}

/// Library version (matches the crate version), for SRI / compatibility checks.
#[wasm_bindgen(js_name = version)]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------------------
// globalThis.digClient installer
// ---------------------------------------------------------------------------

/// On module load, install a `globalThis.digClient` object exposing the read
/// API, so non-bundler consumers (the standalone usercontent loader) can call
/// `globalThis.digClient.verifyInclusion(...)` / `.decryptResourceToText(...)`
/// after the wasm initializes. ES-module consumers can instead import the named
/// functions directly. Idempotent and best-effort (no-op if `globalThis` lacks
/// `Object`, e.g. in a non-browser host).
#[wasm_bindgen(start)]
pub fn install_global() {
    // Build the object from the wasm-bindgen exports via a small JS shim. We use
    // js-sys reflection so this works whether loaded as a classic script (via the
    // generated `--target no-modules` glue) or an ES module.
    let global = js_sys::global();
    let obj = js_sys::Object::new();

    macro_rules! set {
        ($name:literal, $f:expr) => {{
            let closure: JsValue = $f.into_js_value();
            let _ = js_sys::Reflect::set(&obj, &JsValue::from_str($name), &closure);
        }};
    }

    // Expose the high-level entry points the loader uses, plus the primitives.
    set!(
        "reconstructUrn",
        Closure::<dyn Fn(String, String) -> Result<String, JsError>>::new(
            |s: String, r: String| reconstruct_urn(&s, &r)
        )
    );
    set!(
        "retrievalKey",
        Closure::<dyn Fn(String, String) -> Result<String, JsError>>::new(
            |s: String, r: String| retrieval_key(&s, &r)
        )
    );
    set!(
        "deriveKey",
        Closure::<dyn Fn(String, String, Option<String>) -> Result<String, JsError>>::new(
            |s: String, r: String, salt: Option<String>| derive_key(&s, &r, salt)
        )
    );
    set!(
        "verifyInclusion",
        Closure::<dyn Fn(Vec<u8>, String, String) -> Result<bool, JsError>>::new(
            |ct: Vec<u8>, p: String, root: String| verify_inclusion(&ct, &p, &root)
        )
    );
    set!(
        "decryptResource",
        Closure::<
            dyn Fn(
                String,
                String,
                Vec<u8>,
                String,
                String,
                Option<String>,
                Option<Vec<u32>>,
            ) -> Result<Vec<u8>, JsError>,
        >::new(
            |s: String,
             r: String,
             ct: Vec<u8>,
             p: String,
             root: String,
             salt: Option<String>,
             lens: Option<Vec<u32>>| decrypt_resource(
                &s, &r, &ct, &p, &root, salt, lens
            )
        )
    );
    set!(
        "decryptResourceToText",
        Closure::<
            dyn Fn(
                String,
                String,
                Vec<u8>,
                String,
                String,
                Option<String>,
                Option<Vec<u32>>,
            ) -> Result<String, JsError>,
        >::new(
            |s: String,
             r: String,
             ct: Vec<u8>,
             p: String,
             root: String,
             salt: Option<String>,
             lens: Option<Vec<u32>>| decrypt_resource_to_text(
                &s, &r, &ct, &p, &root, salt, lens
            )
        )
    );
    set!("version", Closure::<dyn Fn() -> String>::new(version));

    let _ = js_sys::Reflect::set(&global, &JsValue::from_str("digClient"), &obj);
}
