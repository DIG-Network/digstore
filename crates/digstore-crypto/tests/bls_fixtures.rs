use digstore_crypto::fixtures::BlsFixtureSet;

#[test]
fn fixture_set_self_verifies_and_tags_scheme() {
    let set = BlsFixtureSet::generate();
    assert_eq!(
        set.scheme,
        digstore_crypto::CHIA_BLS_SCHEME,
        "scheme tag must be the shared const"
    );
    assert!(!set.vectors.is_empty(), "must emit at least one vector");
    for v in &set.vectors {
        let pk = digstore_core::Bytes48(hex::decode(&v.pubkey_hex).unwrap().try_into().unwrap());
        let sig = digstore_core::Bytes96(hex::decode(&v.signature_hex).unwrap().try_into().unwrap());
        let msg = hex::decode(&v.message_hex).unwrap();
        assert!(
            digstore_crypto::bls_verify(&pk, &msg, &sig),
            "fixture '{}' must verify under blst signer side",
            v.name
        );
    }
}

#[test]
fn committed_bls_fixture_matches_generated() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bls_vectors.json");
    let on_disk = std::fs::read_to_string(&path).expect(
        "committed bls_vectors.json must exist; run: cargo run -p digstore-crypto --example gen_fixtures",
    );
    let parsed: BlsFixtureSet = serde_json::from_str(&on_disk).unwrap();
    let fresh = BlsFixtureSet::generate();

    assert_eq!(parsed.scheme, fresh.scheme);
    assert_eq!(parsed.vectors.len(), fresh.vectors.len());
    for (a, b) in parsed.vectors.iter().zip(fresh.vectors.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.seed_hex, b.seed_hex);
        assert_eq!(a.message_hex, b.message_hex);
        assert_eq!(a.pubkey_hex, b.pubkey_hex, "pubkey drift in '{}'", a.name);
        assert_eq!(a.signature_hex, b.signature_hex, "sig drift in '{}'", a.name);
    }
}

#[test]
fn write_path_is_idempotent_in_tempdir() {
    // Exercise write_bls_fixtures WITHOUT touching the source tree.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bls_vectors.json");
    digstore_crypto::write_bls_fixtures(&path).expect("write fixtures to tempdir");
    let first = std::fs::read_to_string(&path).unwrap();
    digstore_crypto::write_bls_fixtures(&path).expect("rewrite is deterministic");
    let second = std::fs::read_to_string(&path).unwrap();
    assert_eq!(first, second, "fixture generation must be byte-stable");
}
