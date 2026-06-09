use digstore_core::Bytes48;
use digstore_crypto::{bls, decrypt_and_unwrap, encrypt_chunk, BlsError, CryptoError, TamperError};

#[test]
fn decrypt_and_unwrap_ok_path() {
    let key = [0x21u8; 32];
    let pk = bls::SecretKey::from_seed(&[0x80u8; 32])
        .public_key()
        .to_bytes();
    let ct = encrypt_chunk(&key, b"payload");
    let out = decrypt_and_unwrap(&key, &ct, &pk).expect("valid key + valid pk");
    assert_eq!(out, b"payload");
}

#[test]
fn decrypt_and_unwrap_surfaces_tamper_error() {
    let key = [0x21u8; 32];
    let pk = bls::SecretKey::from_seed(&[0x80u8; 32])
        .public_key()
        .to_bytes();
    let mut ct = encrypt_chunk(&key, b"payload");
    ct[0] ^= 0x01;
    let err = decrypt_and_unwrap(&key, &ct, &pk).unwrap_err();
    assert_eq!(err, CryptoError::Tamper(TamperError));
}

#[test]
fn decrypt_and_unwrap_surfaces_bls_error() {
    let key = [0x21u8; 32];
    let ct = encrypt_chunk(&key, b"payload");
    let bad_pk = Bytes48([0xFFu8; 48]);
    let err = decrypt_and_unwrap(&key, &ct, &bad_pk).unwrap_err();
    assert_eq!(err, CryptoError::Bls(BlsError::InvalidPublicKey));
}
