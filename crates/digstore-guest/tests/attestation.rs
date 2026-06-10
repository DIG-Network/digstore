//! Attestation tests + cross-impl BLS parity (CONVENTIONS C8).
//!
//! The crypto crate emits `tests/fixtures/bls_vectors.json` (blst-signed Chia
//! AugScheme vectors). The guest's pure-Rust `bls12_381` verifier MUST accept
//! every one. We load that file directly (C8) rather than a Rust fixture module.

use digstore_guest::attestation::{
    bls_aug_verify, build_challenge, verify_attestation, AttestationError, TrustedSet,
};
use serde_json::Value;

const BLS_VECTORS_JSON: &str =
    include_str!("../../digstore-crypto/tests/fixtures/bls_vectors.json");

struct Vector {
    pubkey: [u8; 48],
    message: Vec<u8>,
    signature: [u8; 96],
}

fn load_vectors() -> Vec<Vector> {
    let v: Value = serde_json::from_str(BLS_VECTORS_JSON).expect("parse bls_vectors.json");
    let arr = v
        .get("vectors")
        .and_then(Value::as_array)
        .expect("vectors array");
    arr.iter()
        .map(|item| {
            let pk = hex::decode(item["pubkey_hex"].as_str().unwrap()).unwrap();
            let sig = hex::decode(item["signature_hex"].as_str().unwrap()).unwrap();
            let msg = hex::decode(item["message_hex"].as_str().unwrap()).unwrap();
            let mut pubkey = [0u8; 48];
            pubkey.copy_from_slice(&pk);
            let mut signature = [0u8; 96];
            signature.copy_from_slice(&sig);
            Vector {
                pubkey,
                message: msg,
                signature,
            }
        })
        .collect()
}

#[test]
fn guest_verifier_accepts_all_crypto_parity_vectors() {
    let vectors = load_vectors();
    assert!(!vectors.is_empty(), "fixtures must contain vectors");
    for v in &vectors {
        assert!(
            bls_aug_verify(&v.pubkey, &v.message, &v.signature),
            "pure-Rust bls12_381 must accept the blst-signed parity vector"
        );
    }
}

#[test]
fn build_challenge_uses_random_nonce_store_id_time() {
    let store_id = [0xAA; 32];
    let nonce = [0x5Au8; 32];
    let time = 1_700_000_000u64;
    let bytes = build_challenge(nonce, store_id, time);
    // SECURITY.md residual #2: signed message =
    // ATTEST_DST || nonce(32) || store_id(32) || time(u64 BE).
    let t = digstore_core::ATTEST_DST;
    let tl = t.len();
    assert_eq!(bytes.len(), tl + 72);
    assert_eq!(&bytes[0..tl], t);
    assert_eq!(&bytes[tl..tl + 32], &nonce);
    assert_eq!(&bytes[tl + 32..tl + 64], &store_id);
    assert_eq!(&bytes[tl + 64..tl + 72], &time.to_be_bytes());
}

#[test]
fn accepts_valid_host_signature() {
    // Use a parity vector as the host-signed challenge: trusted pubkey, the
    // message bytes (= challenge), and a valid AugScheme G2 signature.
    let v = &load_vectors()[0];
    let trusted = TrustedSet::from_pubkeys(&[v.pubkey]);
    let signed_time = 1_700_000_000u64;
    let now = signed_time + 5; // within freshness window
    let res = verify_attestation(
        &trusted,
        &v.message,
        &v.pubkey,
        &v.signature,
        signed_time,
        now,
    );
    assert!(
        res.is_ok(),
        "valid AugScheme signature from a trusted key must verify"
    );
}

#[test]
fn rejects_tampered_signature() {
    let v = &load_vectors()[0];
    let trusted = TrustedSet::from_pubkeys(&[v.pubkey]);
    let mut bad = v.signature;
    bad[0] ^= 0x01;
    let signed_time = 1_700_000_000u64;
    let now = signed_time + 1;
    let res = verify_attestation(&trusted, &v.message, &v.pubkey, &bad, signed_time, now);
    assert!(matches!(
        res,
        Err(AttestationError::BadSignature) | Err(AttestationError::Malformed)
    ));
}

#[test]
fn rejects_stale_attestation() {
    let v = &load_vectors()[0];
    let trusted = TrustedSet::from_pubkeys(&[v.pubkey]);
    let signed_time = 1_700_000_000u64;
    let now = signed_time + 10_000; // far outside freshness window
    let res = verify_attestation(
        &trusted,
        &v.message,
        &v.pubkey,
        &v.signature,
        signed_time,
        now,
    );
    assert_eq!(res, Err(AttestationError::Stale));
}

#[test]
fn rejects_untrusted_key() {
    let v = &load_vectors()[0];
    let trusted = TrustedSet::from_pubkeys(&[[0u8; 48]]); // some other key
    let signed_time = 1_700_000_000u64;
    let now = signed_time + 1;
    let res = verify_attestation(
        &trusted,
        &v.message,
        &v.pubkey,
        &v.signature,
        signed_time,
        now,
    );
    assert_eq!(res, Err(AttestationError::UntrustedKey));
}
