//! Demo verifier: decode a served `ContentResponse` file and verify its merkle
//! proof against a trusted root (the client-side gate `dighost` never performs).
//! Usage: `verify_served <served.bin> <root-64hex>`.

use digstore_core::{Bytes32, ContentResponse, Decode, Decoder};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("served file path");
    let root_hex = args.next().expect("root 64hex");
    let root = Bytes32::from_hex(&root_hex).expect("valid root hex");

    let bytes = std::fs::read(&path).expect("read served file");
    let mut dec = Decoder::new(&bytes);
    let resp = ContentResponse::decode(&mut dec).expect("decode ContentResponse");

    let leaf_ok = resp.merkle_proof.leaf == digstore_crypto::sha256(&resp.ciphertext);
    let verifies = resp.merkle_proof.verify();
    let root_matches = resp.merkle_proof.root == root;
    println!("served bytes        = {}", bytes.len());
    println!("ciphertext bytes    = {}", resp.ciphertext.len());
    println!("response roothash   = {}", resp.roothash.to_hex());
    println!("proof.leaf==sha256  = {leaf_ok}");
    println!("proof.verify()      = {verifies}");
    println!("proof.root==trusted = {root_matches}");
    println!(
        "VERDICT             = {}",
        if leaf_ok && verifies && root_matches {
            "PROOF VERIFIES TO TRUSTED ROOT"
        } else {
            "NOT VERIFIED"
        }
    );
}
