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

#[test]
fn sign_then_verify_round_trip_methods() {
    let sk = bls::SecretKey::from_seed(&[0x10u8; 32]);
    let pk = sk.public_key();
    let msg = b"digstore execution proof payload";
    let sig = sk.sign(msg);
    assert!(pk.verify(msg, &sig), "valid signature must verify (method API)");

    // Byte round-trip of the signature and public key (C1 to_bytes/from_bytes).
    let sig2 = bls::Signature::from_bytes(&sig.to_bytes()).expect("sig bytes round-trip");
    let pk2 = bls::PublicKey::from_bytes(&pk.to_bytes()).expect("pk bytes round-trip");
    assert!(pk2.verify(msg, &sig2), "byte-roundtripped key/sig must verify");
}

#[test]
fn sign_then_verify_round_trip_free_helpers() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, pk) = bls::bls_keygen(&[0x10u8; 32]);
    let msg = b"digstore execution proof payload";
    let sig = bls_sign(&sk, msg);
    assert!(bls_verify(&pk, msg, &sig), "valid signature must verify (free fn)");
}

#[test]
fn verify_rejects_wrong_public_key() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, _pk) = bls::bls_keygen(&[0x20u8; 32]);
    let (_sk2, other_pk) = bls::bls_keygen(&[0x21u8; 32]);
    let msg = b"message";
    let sig = bls_sign(&sk, msg);
    assert!(!bls_verify(&other_pk, msg, &sig), "wrong key must not verify");
}

#[test]
fn verify_rejects_wrong_message() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, pk) = bls::bls_keygen(&[0x30u8; 32]);
    let sig = bls_sign(&sk, b"original");
    assert!(!bls_verify(&pk, b"tampered", &sig), "altered message must not verify");
}

#[test]
fn verify_rejects_malformed_signature_bytes() {
    use digstore_core::Bytes96;
    use digstore_crypto::bls_verify;
    let (_sk, pk) = bls::bls_keygen(&[0x40u8; 32]);
    let bogus = Bytes96([0xFFu8; 96]);
    assert!(!bls_verify(&pk, b"x", &bogus), "non-canonical sig bytes must not verify");
    // And the typed from_bytes path rejects too.
    assert!(bls::Signature::from_bytes(&bogus).is_err());
}

#[test]
fn verify_rejects_malformed_public_key_bytes() {
    use digstore_core::{Bytes48, Bytes96};
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, _pk) = bls::bls_keygen(&[0x41u8; 32]);
    let sig = bls_sign(&sk, b"x");
    let bogus_pk = Bytes48([0xFFu8; 48]);
    let sig96 = Bytes96(sig.0);
    assert!(!bls_verify(&bogus_pk, b"x", &sig96), "non-canonical pk bytes must not verify");
}

#[test]
fn chia_aug_scheme_known_vector() {
    use digstore_core::{Bytes48, Bytes96};

    // Seed = [0, 1, 2, ..., 31].
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = i as u8;
    }
    let sk = bls::SecretKey::from_seed(&seed);
    let pk = sk.public_key();

    // Real chia-bls 0.45 AugScheme reference values for this seed.
    let expected_pk = hex::decode(
        "8f336467f057b373bb3c43815a10ec131119d1bf50c14fa3f9ad86c0ec074f920f936a5315a8365a37fee0afa34c32c6",
    )
    .unwrap();
    assert_eq!(&pk.to_bytes().0[..], &expected_pk[..], "G1 pubkey must match Chia reference");

    let msg = [7u8, 8, 9];
    let sig = sk.sign(&msg);
    let expected_sig = hex::decode(
        "a5ce62a76c749a06c85b2d3762523b2e1d6756455767d2023967480433f7225c5cf42b3e14d0df41c0e6f9ecc18a39c30fdbfdbfd422945b478cc1675adf046aefbf4810e3ab9b0eb09855d3e5540cb0924e0f3d0e324bb59c59659b1c6b4283",
    )
    .unwrap();
    assert_eq!(&sig.to_bytes().0[..], &expected_sig[..], "AugScheme G2 sig must match Chia reference");

    // The frozen vector must self-verify through our verifier.
    let pk48 = Bytes48(expected_pk.try_into().unwrap());
    let sig96 = Bytes96(expected_sig.try_into().unwrap());
    assert!(digstore_crypto::bls_verify(&pk48, &msg, &sig96));
    // And must NOT verify a different message (binding sanity).
    assert!(!digstore_crypto::bls_verify(&pk48, &[9u8, 9, 9], &sig96));
}
