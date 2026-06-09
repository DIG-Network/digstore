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
    let p1 = bls::SecretKey::from_seed(&[0x01u8; 32])
        .public_key()
        .to_bytes();
    let p2 = bls::SecretKey::from_seed(&[0x02u8; 32])
        .public_key()
        .to_bytes();
    assert_ne!(p1, p2);
}

#[test]
fn from_bytes_rejects_non_canonical_public_key() {
    use digstore_core::Bytes48;
    use digstore_crypto::CryptoError;
    let bogus = Bytes48([0xFFu8; 48]);
    let err = bls::PublicKey::from_bytes(&bogus)
        .err()
        .expect("must reject");
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
    assert!(
        pk.verify(msg, &sig),
        "valid signature must verify (method API)"
    );

    // Byte round-trip of the signature and public key (C1 to_bytes/from_bytes).
    let sig2 = bls::Signature::from_bytes(&sig.to_bytes()).expect("sig bytes round-trip");
    let pk2 = bls::PublicKey::from_bytes(&pk.to_bytes()).expect("pk bytes round-trip");
    assert!(
        pk2.verify(msg, &sig2),
        "byte-roundtripped key/sig must verify"
    );
}

#[test]
fn sign_then_verify_round_trip_free_helpers() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, pk) = bls::bls_keygen(&[0x10u8; 32]);
    let msg = b"digstore execution proof payload";
    let sig = bls_sign(&sk, msg);
    assert!(
        bls_verify(&pk, msg, &sig),
        "valid signature must verify (free fn)"
    );
}

#[test]
fn verify_rejects_wrong_public_key() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, _pk) = bls::bls_keygen(&[0x20u8; 32]);
    let (_sk2, other_pk) = bls::bls_keygen(&[0x21u8; 32]);
    let msg = b"message";
    let sig = bls_sign(&sk, msg);
    assert!(
        !bls_verify(&other_pk, msg, &sig),
        "wrong key must not verify"
    );
}

#[test]
fn verify_rejects_wrong_message() {
    use digstore_crypto::{bls_sign, bls_verify};
    let (sk, pk) = bls::bls_keygen(&[0x30u8; 32]);
    let sig = bls_sign(&sk, b"original");
    assert!(
        !bls_verify(&pk, b"tampered", &sig),
        "altered message must not verify"
    );
}

#[test]
fn verify_rejects_malformed_signature_bytes() {
    use digstore_core::Bytes96;
    use digstore_crypto::bls_verify;
    let (_sk, pk) = bls::bls_keygen(&[0x40u8; 32]);
    let bogus = Bytes96([0xFFu8; 96]);
    assert!(
        !bls_verify(&pk, b"x", &bogus),
        "non-canonical sig bytes must not verify"
    );
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
    assert!(
        !bls_verify(&bogus_pk, b"x", &sig96),
        "non-canonical pk bytes must not verify"
    );
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
    assert_eq!(
        &pk.to_bytes().0[..],
        &expected_pk[..],
        "G1 pubkey must match Chia reference"
    );

    let msg = [7u8, 8, 9];
    let sig = sk.sign(&msg);
    let expected_sig = hex::decode(
        "a5ce62a76c749a06c85b2d3762523b2e1d6756455767d2023967480433f7225c5cf42b3e14d0df41c0e6f9ecc18a39c30fdbfdbfd422945b478cc1675adf046aefbf4810e3ab9b0eb09855d3e5540cb0924e0f3d0e324bb59c59659b1c6b4283",
    )
    .unwrap();
    assert_eq!(
        &sig.to_bytes().0[..],
        &expected_sig[..],
        "AugScheme G2 sig must match Chia reference"
    );

    // The frozen vector must self-verify through our verifier.
    let pk48 = Bytes48(expected_pk.try_into().unwrap());
    let sig96 = Bytes96(expected_sig.try_into().unwrap());
    assert!(digstore_crypto::bls_verify(&pk48, &msg, &sig96));
    // And must NOT verify a different message (binding sanity).
    assert!(!digstore_crypto::bls_verify(&pk48, &[9u8, 9, 9], &sig96));
}

#[test]
fn sign_push_then_verify_push_round_trip_and_binding() {
    use digstore_core::Bytes32;
    use digstore_crypto::{push_signing_message, sha256, sign_push, verify_push};

    let sk = bls::SecretKey::from_seed(&[0x50u8; 32]);
    let pk = sk.public_key();
    let root = Bytes32([0xAAu8; 32]);
    let store_id = Bytes32([0xBBu8; 32]);

    let sig = sign_push(&sk, &root, &store_id);
    assert!(
        verify_push(&pk, &root, &store_id, &sig),
        "push sig must verify with verify_push"
    );

    // CONVENTIONS C7: the signed message is SHA-256(root || store_id) (32 bytes).
    let mut concat = Vec::new();
    concat.extend_from_slice(&root.0);
    concat.extend_from_slice(&store_id.0);
    let expected = sha256(&concat).0;
    assert_eq!(push_signing_message(&root, &store_id), expected);
    assert_eq!(push_signing_message(&root, &store_id).len(), 32);

    // Wrong store_id must not verify (binding to store).
    let other_store = Bytes32([0xCCu8; 32]);
    assert!(!verify_push(&pk, &root, &other_store, &sig));
    // Wrong root must not verify.
    let other_root = Bytes32([0xDDu8; 32]);
    assert!(!verify_push(&pk, &other_root, &store_id, &sig));
    // Signing over the RAW concat (not its hash) would not verify.
    assert!(!digstore_crypto::bls_verify(&pk.to_bytes(), &concat, &sig));
}

#[test]
fn sign_node_binds_program_output_anchor_and_input() {
    use digstore_core::Bytes32;
    use digstore_crypto::{bls_verify, node_signing_message, sign_node};

    let sk = bls::SecretKey::from_seed(&[0x60u8; 32]);
    let pk = sk.public_key().to_bytes();
    let program_hash = Bytes32([0x01u8; 32]);
    let public_output = Bytes32([0x02u8; 32]);
    let header_hash = Bytes32([0x03u8; 32]);
    let height: u32 = 0x00ABCDEF;
    let public_input = vec![9u8, 8, 7];

    let sig = sign_node(
        &sk,
        &program_hash,
        &public_output,
        &header_hash,
        height,
        &public_input,
    );

    // Verifies against the canonical message.
    let msg = node_signing_message(
        &program_hash,
        &public_output,
        &header_hash,
        height,
        &public_input,
    );
    assert!(bls_verify(&pk, &msg, &sig));

    // height is big-endian: a different height must not verify.
    let wrong_height = node_signing_message(
        &program_hash,
        &public_output,
        &header_hash,
        height + 1,
        &public_input,
    );
    assert!(!bls_verify(&pk, &wrong_height, &sig));

    // Changing the bound output must not verify.
    let other_output = Bytes32([0x99u8; 32]);
    let wrong_out = node_signing_message(
        &program_hash,
        &other_output,
        &header_hash,
        height,
        &public_input,
    );
    assert!(!bls_verify(&pk, &wrong_out, &sig));

    // Changing the anchor (header_hash) must not verify.
    let other_anchor = Bytes32([0x77u8; 32]);
    let wrong_anchor = node_signing_message(
        &program_hash,
        &public_output,
        &other_anchor,
        height,
        &public_input,
    );
    assert!(!bls_verify(&pk, &wrong_anchor, &sig));
}

#[test]
fn node_signing_message_layout_is_exact() {
    use digstore_core::Bytes32;
    use digstore_crypto::node_signing_message;
    let pg = Bytes32([0x01u8; 32]);
    let out = Bytes32([0x02u8; 32]);
    let hdr = Bytes32([0x03u8; 32]);
    let height: u32 = 0x01020304;
    let pi = vec![0xEE, 0xFF];
    let msg = node_signing_message(&pg, &out, &hdr, height, &pi);
    // 32 + 32 + 32 + 4 + 2 = 102 bytes.
    assert_eq!(msg.len(), 102);
    assert_eq!(&msg[0..32], &[0x01u8; 32]);
    assert_eq!(&msg[32..64], &[0x02u8; 32]);
    assert_eq!(&msg[64..96], &[0x03u8; 32]);
    assert_eq!(&msg[96..100], &[0x01, 0x02, 0x03, 0x04]); // big-endian height
    assert_eq!(&msg[100..102], &[0xEE, 0xFF]);
}

#[test]
fn sign_attestation_binds_nonce_store_and_timestamp() {
    use digstore_core::AttestationChallenge;
    use digstore_crypto::{attestation_signing_message, bls_verify, sign_attestation};

    let sk = bls::SecretKey::from_seed(&[0x70u8; 32]);
    let pk = sk.public_key().to_bytes();
    let challenge = AttestationChallenge {
        nonce: [0x5A; 32],
        store_id: [0x6B; 32],
        timestamp: 0x0102_0304_0506_0708,
    };

    let sig = sign_attestation(&sk, &challenge);

    let msg =
        attestation_signing_message(&challenge.nonce, &challenge.store_id, challenge.timestamp);
    assert!(bls_verify(&pk, &msg, &sig));

    // Layout: 32 + 32 + 8 = 72 bytes, timestamp big-endian.
    assert_eq!(msg.len(), 72);
    assert_eq!(&msg[0..32], &[0x5Au8; 32]);
    assert_eq!(&msg[32..64], &[0x6Bu8; 32]);
    assert_eq!(
        &msg[64..72],
        &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    );

    // A different nonce must not verify.
    let wrong = attestation_signing_message(&[0x00; 32], &challenge.store_id, challenge.timestamp);
    assert!(!bls_verify(&pk, &wrong, &sig));
}
