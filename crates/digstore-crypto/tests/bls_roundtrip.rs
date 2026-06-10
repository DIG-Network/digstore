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

    // CONVENTIONS C7 + SECURITY.md residual #2: the signed message is now
    // SHA-256(PUSH_DST || root || store_id) (still 32 bytes; the per-role tag is
    // folded into the hashed preimage).
    let mut concat = Vec::new();
    concat.extend_from_slice(digstore_crypto::bls::PUSH_DST);
    concat.extend_from_slice(&root.0);
    concat.extend_from_slice(&store_id.0);
    let expected = sha256(&concat).0;
    assert_eq!(push_signing_message(&root, &store_id), expected);
    assert_eq!(push_signing_message(&root, &store_id).len(), 32);
    // The role tag actually changes the message vs the untagged preimage.
    let mut untagged = Vec::new();
    untagged.extend_from_slice(&root.0);
    untagged.extend_from_slice(&store_id.0);
    assert_ne!(push_signing_message(&root, &store_id), sha256(&untagged).0);

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
    // SECURITY.md residual #2: NODE_DST || 32 + 32 + 32 + 4 + 2.
    let t = digstore_crypto::bls::NODE_DST;
    let tl = t.len();
    assert_eq!(msg.len(), tl + 102);
    assert_eq!(&msg[0..tl], t);
    assert_eq!(&msg[tl..tl + 32], &[0x01u8; 32]);
    assert_eq!(&msg[tl + 32..tl + 64], &[0x02u8; 32]);
    assert_eq!(&msg[tl + 64..tl + 96], &[0x03u8; 32]);
    assert_eq!(&msg[tl + 96..tl + 100], &[0x01, 0x02, 0x03, 0x04]); // big-endian height
    assert_eq!(&msg[tl + 100..tl + 102], &[0xEE, 0xFF]);
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

    // Layout (SECURITY.md residual #2): ATTEST_DST || 32 + 32 + 8, timestamp big-endian.
    let t = digstore_core::ATTEST_DST;
    let tl = t.len();
    assert_eq!(msg.len(), tl + 72);
    assert_eq!(&msg[0..tl], t);
    assert_eq!(&msg[tl..tl + 32], &[0x5Au8; 32]);
    assert_eq!(&msg[tl + 32..tl + 64], &[0x6Bu8; 32]);
    assert_eq!(
        &msg[tl + 64..tl + 72],
        &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    );

    // A different nonce must not verify.
    let wrong = attestation_signing_message(&[0x00; 32], &challenge.store_id, challenge.timestamp);
    assert!(!bls_verify(&pk, &wrong, &sig));
}

#[test]
fn role_tags_domain_separate_identical_payloads() {
    // SECURITY.md residual #2: the three canonical builders carry DISTINCT
    // per-role tags, so a signature minted for one role cannot be replayed as a
    // signature for another even when the underlying payload bytes coincide.
    use digstore_core::Bytes32;
    use digstore_crypto::{
        attestation_signing_message, bls_verify, node_signing_message, push_signing_message,
        sign_attestation, sign_node, sign_push,
    };

    // The three role tags are pairwise distinct constants.
    assert_ne!(digstore_crypto::bls::PUSH_DST, digstore_crypto::bls::NODE_DST);
    assert_ne!(digstore_crypto::bls::PUSH_DST, digstore_core::ATTEST_DST);
    assert_ne!(digstore_crypto::bls::NODE_DST, digstore_core::ATTEST_DST);

    // Construct a push message and an attestation message over the SAME 64 payload
    // bytes (nonce||store_id vs root||store_id), then confirm the produced messages
    // differ purely because of the role tags.
    let a = Bytes32([0x5Au8; 32]);
    let b = Bytes32([0x6Bu8; 32]);
    let push_msg = push_signing_message(&a, &b); // SHA-256(PUSH_DST || a || b)
    let attest_msg = attestation_signing_message(&[0x5A; 32], &[0x6B; 32], 0);
    // The attestation message embeds ATTEST_DST as its literal prefix; the push
    // message hashes PUSH_DST in. They can never be byte-equal (different lengths
    // and different leading bytes), so cross-role reuse is structurally impossible.
    assert_ne!(&attest_msg[..push_msg.len().min(attest_msg.len())], &push_msg[..]);
    assert_eq!(&attest_msg[..digstore_core::ATTEST_DST.len()], digstore_core::ATTEST_DST);

    // End-to-end: a push signature must NOT verify as a node-proof or attestation
    // signature, and vice versa — even with deliberately aligned inputs.
    let sk = bls::SecretKey::from_seed(&[0x80u8; 32]);
    let pk = sk.public_key().to_bytes();

    let root = Bytes32([0x11u8; 32]);
    let store = Bytes32([0x22u8; 32]);
    let push_sig = sign_push(&sk, &root, &store);
    // verifies only against the push message
    assert!(verify_push_ok(&sk, &root, &store, &push_sig));
    // does NOT verify against a node message built from the same 32-byte halves
    let node_msg = node_signing_message(&root, &store, &Bytes32([0u8; 32]), 0, &[]);
    assert!(!bls_verify(&pk, &node_msg, &push_sig));

    let node_sig = sign_node(&sk, &root, &store, &Bytes32([0u8; 32]), 0, &[]);
    let attest_over_same = attestation_signing_message(&root.0, &store.0, 0);
    assert!(!bls_verify(&pk, &attest_over_same, &node_sig));

    let challenge = digstore_core::AttestationChallenge {
        nonce: [0x11u8; 32],
        store_id: [0x22u8; 32],
        timestamp: 0,
    };
    let attest_sig = sign_attestation(&sk, &challenge);
    let push_over_same = push_signing_message(&root, &store);
    assert!(!bls_verify(&pk, &push_over_same, &attest_sig));
}

#[test]
fn sign_tombstone_then_verify_round_trip_and_binding() {
    use digstore_core::tombstone::{RevocationReason, Tombstone};
    use digstore_core::Bytes32;
    use digstore_crypto::{sha256, sign_tombstone, tombstone_signing_message, verify_tombstone};

    let sk = bls::SecretKey::from_seed(&[0x90u8; 32]);
    let pk = sk.public_key();
    let store_id = Bytes32([0xA1u8; 32]);
    let root = Bytes32([0xB2u8; 32]);
    let t = Tombstone::root(store_id, root, 1_700_000_000, RevocationReason::Compromise);

    let sig = sign_tombstone(&sk, &t);
    assert!(
        verify_tombstone(&pk, &t, &sig),
        "tombstone sig must verify with verify_tombstone"
    );

    // The signed message is SHA-256(TOMB_DST || canonical(t)) (32 bytes).
    let mut concat = Vec::new();
    concat.extend_from_slice(digstore_crypto::bls::TOMB_DST);
    concat.extend_from_slice(&t.canonical());
    let expected = sha256(&concat).0;
    assert_eq!(tombstone_signing_message(&t), expected);
    assert_eq!(tombstone_signing_message(&t).len(), 32);

    // Tamper: any altered field changes the message and the sig must not verify.
    let tampered_root =
        Tombstone::root(store_id, Bytes32([0xC3u8; 32]), 1_700_000_000, RevocationReason::Compromise);
    assert!(!verify_tombstone(&pk, &tampered_root, &sig));
    let tampered_reason =
        Tombstone::root(store_id, root, 1_700_000_000, RevocationReason::Takedown);
    assert!(!verify_tombstone(&pk, &tampered_reason, &sig));
    let tampered_store =
        Tombstone::root(Bytes32([0xFFu8; 32]), root, 1_700_000_000, RevocationReason::Compromise);
    assert!(!verify_tombstone(&pk, &tampered_store, &sig));

    // Wrong key must not verify.
    let other = bls::SecretKey::from_seed(&[0x91u8; 32]).public_key();
    assert!(!verify_tombstone(&other, &t, &sig));

    // Malformed signature bytes fail closed.
    let bogus = digstore_core::Bytes96([0xFFu8; 96]);
    assert!(!verify_tombstone(&pk, &t, &bogus));
}

#[test]
fn tombstone_message_domain_separated_from_other_roles() {
    // SECURITY.md residual #1 Layer 1: the tombstone tag is distinct from the
    // push/node/attestation tags, so a tombstone signature can never be replayed
    // as (nor forged from) one of those, even over coinciding payload bytes.
    use digstore_core::tombstone::{RevocationReason, Tombstone};
    use digstore_core::Bytes32;
    use digstore_crypto::{
        attestation_signing_message, bls_verify, node_signing_message, push_signing_message,
        sign_tombstone, tombstone_signing_message,
    };

    // The tombstone tag is pairwise distinct from every other role tag.
    let tomb = digstore_crypto::bls::TOMB_DST;
    assert_ne!(tomb, digstore_crypto::bls::PUSH_DST);
    assert_ne!(tomb, digstore_crypto::bls::NODE_DST);
    assert_ne!(tomb, digstore_core::ATTEST_DST);

    let store_id = Bytes32([0x22u8; 32]);
    let root = Bytes32([0x11u8; 32]);
    let t = Tombstone::root(store_id, root, 0, RevocationReason::Unspecified);

    // Build push/node/attestation messages over the SAME (root, store_id) bytes
    // and confirm none equals the tombstone message: the role tag separates them.
    let tomb_msg = tombstone_signing_message(&t);
    let push_msg = push_signing_message(&root, &store_id);
    let node_msg = node_signing_message(&root, &store_id, &Bytes32([0u8; 32]), 0, &[]);
    let attest_msg = attestation_signing_message(&root.0, &store_id.0, 0);
    assert_ne!(tomb_msg.as_slice(), push_msg.as_slice());
    assert_ne!(tomb_msg.as_slice(), node_msg.as_slice());
    assert_ne!(tomb_msg.as_slice(), attest_msg.as_slice());

    // End-to-end: a tombstone signature must NOT verify as a push/node/attestation
    // signature over the aligned inputs, and vice versa.
    let sk = bls::SecretKey::from_seed(&[0x92u8; 32]);
    let pk = sk.public_key().to_bytes();
    let tomb_sig = sign_tombstone(&sk, &t);
    assert!(!bls_verify(&pk, &push_msg, &tomb_sig));
    assert!(!bls_verify(&pk, &node_msg, &tomb_sig));
    assert!(!bls_verify(&pk, &attest_msg, &tomb_sig));

    // A push signature must not verify against the tombstone message either.
    let push_sig = digstore_crypto::sign_push(&sk, &root, &store_id);
    assert!(!bls_verify(&pk, &tomb_msg, &push_sig));
}

// Local helper: verify a push signature via the canonical message.
fn verify_push_ok(
    sk: &bls::SecretKey,
    root: &digstore_core::Bytes32,
    store: &digstore_core::Bytes32,
    sig: &digstore_core::Bytes96,
) -> bool {
    digstore_crypto::verify_push(&sk.public_key(), root, store, sig)
}
