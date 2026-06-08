use digstore_crypto::bls;

#[test]
fn keygen_is_deterministic_and_pubkey_validates() {
    let seed = [0xABu8; 32];
    let sk1 = bls::SecretKey::from_seed(&seed);
    let sk2 = bls::SecretKey::from_seed(&seed);
    let pk1 = sk1.public_key().to_bytes();
    let pk2 = sk2.public_key().to_bytes();
    assert_eq!(pk1, pk2, "same seed must yield same public key");
    assert_ne!(pk1.0, [0u8; 48], "public key must not be all-zero");
    // Round-trip the public key bytes back into a PublicKey (canonical G1).
    assert!(
        bls::PublicKey::from_bytes(&pk1).is_ok(),
        "keygen output must be a valid G1 point"
    );
}

#[test]
fn distinct_seeds_yield_distinct_pubkeys() {
    let p1 = bls::SecretKey::from_seed(&[0x01u8; 32]).public_key().to_bytes();
    let p2 = bls::SecretKey::from_seed(&[0x02u8; 32]).public_key().to_bytes();
    assert_ne!(p1, p2);
}

#[test]
fn from_bytes_rejects_non_canonical_public_key() {
    use digstore_core::Bytes48;
    use digstore_crypto::CryptoError;
    let bogus = Bytes48([0xFFu8; 48]);
    let err = bls::PublicKey::from_bytes(&bogus).err().expect("must reject");
    assert_eq!(
        err,
        CryptoError::Bls(digstore_crypto::BlsError::InvalidPublicKey)
    );
}

#[test]
fn aliases_resolve_to_the_real_types() {
    // CONVENTIONS C1: BlsSecretKey / BlsPublicKey aliases must exist.
    let sk: bls::BlsSecretKey = bls::SecretKey::from_seed(&[0x05u8; 32]);
    let _pk: bls::BlsPublicKey = sk.public_key();
}
