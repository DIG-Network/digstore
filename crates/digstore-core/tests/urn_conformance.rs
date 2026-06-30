//! Cross-implementation URN CONFORMANCE VECTORS (gap #128).
//!
//! `tests/fixtures/urn_conformance.json` is the FROZEN, normative vector file: a set
//! of canonical input URNs → their expected parsed fields, canonical re-emission, and
//! derived `retrieval_key = SHA-256(canonical())`, plus a list of inputs that MUST be
//! rejected. It is the single source of truth every Digstore URN parser
//! (digstore-core here, plus the dig-sdk / extension / browser ports) is expected to
//! conform to — the same way the KDF KAT (`crypto/tests/fixtures/kdf_kat.json`) and
//! the merkle goldens are frozen.
//!
//! This test PROVES `digstore_core`'s parser conforms to the frozen vectors and is the
//! drift guard: if `Urn::parse` / `canonical()` / `retrieval_key()` ever change shape,
//! one of these assertions fails. The grammar the vectors encode is documented in
//! `digstore_core::urn_grammar` (the normative ABNF).
//!
//! To regenerate the fixture after an INTENTIONAL, reviewed grammar change, run:
//!   `cargo test -p digstore-core --test urn_conformance -- --ignored regenerate`
//! and commit the updated JSON (the regenerator recomputes every retrieval key from
//! the canonical form, so the goldens can never be hand-mistyped).

use digstore_core::sha256;
use digstore_core::urn::Urn;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct VectorSet {
    /// The normative chain the canonical form SHOULD carry. Asserted == core::CHAIN.
    canonical_chain: String,
    valid: Vec<ValidVector>,
    invalid: Vec<InvalidVector>,
}

#[derive(Debug, Deserialize)]
struct ValidVector {
    /// Short identifier for failure messages.
    name: String,
    /// The input URN to parse.
    input: String,
    /// Expected parsed `chain`.
    chain: String,
    /// Expected parsed `store_id` (lowercase hex).
    store_id_hex: String,
    /// Expected parsed `root_hash` (lowercase hex), or `null` when absent.
    root_hash_hex: Option<String>,
    /// Expected parsed `resource_key`, or `null` when absent (note: an empty string
    /// `""` is DISTINCT from absent — a trailing `/`).
    resource_key: Option<String>,
    /// Expected `canonical()` re-emission.
    canonical: String,
    /// Expected `retrieval_key = SHA-256(canonical())`, lowercase hex.
    retrieval_key_hex: String,
}

#[derive(Debug, Deserialize)]
struct InvalidVector {
    name: String,
    input: String,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("urn_conformance.json")
}

fn load() -> VectorSet {
    let raw =
        std::fs::read_to_string(fixture_path()).expect("read tests/fixtures/urn_conformance.json");
    serde_json::from_str(&raw).expect("parse urn_conformance.json")
}

#[test]
fn fixture_chain_matches_canonical_chain_constant() {
    let set = load();
    assert_eq!(
        set.canonical_chain,
        digstore_core::CHAIN,
        "the fixture's canonical chain must equal digstore_core::CHAIN"
    );
    assert_eq!(
        set.canonical_chain,
        digstore_core::urn_grammar::CANONICAL_CHAIN,
        "the grammar module's CANONICAL_CHAIN must equal the fixture's canonical chain"
    );
}

#[test]
fn valid_vectors_parse_to_the_expected_fields() {
    let set = load();
    assert!(!set.valid.is_empty(), "expected at least one valid vector");
    for v in &set.valid {
        let urn = Urn::parse(&v.input)
            .unwrap_or_else(|e| panic!("[{}] valid URN failed to parse: {e:?}", v.name));
        assert_eq!(urn.chain, v.chain, "[{}] chain", v.name);
        assert_eq!(
            urn.store_id.to_hex(),
            v.store_id_hex,
            "[{}] store_id",
            v.name
        );
        assert_eq!(
            urn.root_hash.map(|h| h.to_hex()),
            v.root_hash_hex,
            "[{}] root_hash",
            v.name
        );
        assert_eq!(
            urn.resource_key, v.resource_key,
            "[{}] resource_key",
            v.name
        );
    }
}

#[test]
fn valid_vectors_canonicalize_and_derive_the_frozen_retrieval_key() {
    let set = load();
    for v in &set.valid {
        let urn = Urn::parse(&v.input).expect("valid URN parses");
        assert_eq!(urn.canonical(), v.canonical, "[{}] canonical()", v.name);
        // retrieval_key = SHA-256(canonical()) — the frozen golden.
        let rk = urn.retrieval_key().to_hex();
        assert_eq!(rk, v.retrieval_key_hex, "[{}] retrieval_key", v.name);
        // And it is exactly SHA-256 of the canonical bytes (the normative definition).
        assert_eq!(
            rk,
            sha256(v.canonical.as_bytes()).to_hex(),
            "[{}] retrieval_key must be SHA-256(canonical)",
            v.name
        );
    }
}

#[test]
fn canonical_input_is_idempotent_under_reparse() {
    let set = load();
    for v in &set.valid {
        let urn = Urn::parse(&v.input).expect("valid URN parses");
        let canon = urn.canonical();
        let reparsed = Urn::parse(&canon).expect("canonical re-parses");
        assert_eq!(reparsed.canonical(), canon, "[{}] re-canonical", v.name);
        assert_eq!(
            reparsed.retrieval_key(),
            urn.retrieval_key(),
            "[{}] re-retrieval_key",
            v.name
        );
    }
}

#[test]
fn invalid_vectors_are_rejected() {
    let set = load();
    assert!(
        !set.invalid.is_empty(),
        "expected at least one invalid vector"
    );
    for v in &set.invalid {
        assert!(
            Urn::parse(&v.input).is_err(),
            "[{}] expected rejection but parsed: {:?}",
            v.name,
            v.input
        );
    }
}

/// Regenerate `urn_conformance.json` from the SAME input list, recomputing every
/// canonical form + retrieval key from the live parser so the goldens can never be
/// hand-mistyped. Run explicitly after an intentional, reviewed grammar change:
///   `cargo test -p digstore-core --test urn_conformance -- --ignored regenerate`
/// Then review the diff and commit. NOT part of the normal test run.
#[test]
#[ignore = "regenerate the frozen fixture on demand only"]
fn regenerate() {
    // (name, input) for valid vectors — chain/fields/canonical/retrieval are derived.
    let valid_inputs: &[(&str, &str)] = &[
        (
            "chia_full_root_and_resource",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222/index.html",
        ),
        (
            "chia_store_only_bare",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111",
        ),
        (
            "chia_store_and_resource_no_root",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/path/to/file.txt",
        ),
        (
            "chia_root_no_resource",
            "urn:dig:chia:aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        ),
        (
            "chia_trailing_slash_empty_resource",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/",
        ),
        (
            "chia_nested_resource_path",
            "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000:0000000000000000000000000000000000000000000000000000000000000000/a/b/c.json",
        ),
        // Multi-chain corpus reality (KDF KAT uses these): core accepts them.
        (
            "mainnet_label_accepted",
            "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/a",
        ),
        (
            "testnet_label_bare_store",
            "urn:dig:testnet:1111111111111111111111111111111111111111111111111111111111111111",
        ),
        // A query-looking suffix is NOT a salt: it lands verbatim in the resource.
        (
            "query_suffix_is_part_of_resource_not_salt",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/index.html?salt=deadbeef",
        ),
    ];

    let valid: Vec<serde_json::Value> = valid_inputs
        .iter()
        .map(|(name, input)| {
            let urn = Urn::parse(input).expect("regenerate: input must be valid");
            serde_json::json!({
                "name": name,
                "input": input,
                "chain": urn.chain,
                "store_id_hex": urn.store_id.to_hex(),
                "root_hash_hex": urn.root_hash.map(|h| h.to_hex()),
                "resource_key": urn.resource_key,
                "canonical": urn.canonical(),
                "retrieval_key_hex": urn.retrieval_key().to_hex(),
            })
        })
        .collect();

    let invalid: Vec<serde_json::Value> = [
        ("wrong_scheme_other", "urn:other:chia:00"),
        ("not_a_urn", "not-a-urn"),
        ("missing_store_id", "urn:dig:chia"),
        ("empty_chain", "urn:dig::1111111111111111111111111111111111111111111111111111111111111111"),
        ("store_id_not_hex", "urn:dig:chia:zzzz"),
        ("store_id_too_short", "urn:dig:chia:1111"),
        ("root_hash_not_hex", "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111:zz"),
        ("too_many_colon_segments", "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222:3333333333333333333333333333333333333333333333333333333333333333"),
        ("missing_prefix_dig", "dig:chia:1111111111111111111111111111111111111111111111111111111111111111"),
    ]
    .iter()
    .map(|(name, input)| serde_json::json!({ "name": name, "input": input }))
    .collect();

    let doc = serde_json::json!({
        "_comment": "FROZEN normative URN conformance vectors (gap #128). Grammar: digstore_core::urn_grammar (URN_ABNF). Regenerate via `cargo test -p digstore-core --test urn_conformance -- --ignored regenerate`. retrieval_key_hex = SHA-256(canonical) lowercase hex.",
        "canonical_chain": digstore_core::CHAIN,
        "valid": valid,
        "invalid": invalid,
    });

    let pretty = serde_json::to_string_pretty(&doc).expect("serialize fixture");
    std::fs::write(fixture_path(), pretty + "\n").expect("write fixture");
    eprintln!("regenerated {}", fixture_path().display());
}
