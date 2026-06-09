use digstore_crypto::{decrypt_chunk, derive_decryption_key, encrypt_chunk};

#[test]
fn encrypt_then_decrypt_recovers_plaintext() {
    let key = derive_decryption_key(
        "urn:dig:mainnet:3333333333333333333333333333333333333333333333333333333333333333/x",
        None,
    );
    let plaintext = b"the quick brown fox jumps over the lazy dog".to_vec();
    let ct = encrypt_chunk(&key, &plaintext);
    assert_ne!(ct, plaintext, "ciphertext must differ from plaintext");
    assert_eq!(
        ct.len(),
        plaintext.len() + 16,
        "ct must carry a 16-byte GCM tag"
    );
    let recovered = decrypt_chunk(&key, &ct).expect("authentic ciphertext must decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn empty_plaintext_roundtrips() {
    let key = [0x42u8; 32];
    let ct = encrypt_chunk(&key, b"");
    assert_eq!(ct.len(), 16, "empty plaintext yields just the 16-byte tag");
    let recovered = decrypt_chunk(&key, &ct).expect("authentic empty ciphertext decrypts");
    assert!(recovered.is_empty());
}

use digstore_crypto::TamperError;

#[test]
fn flipping_a_ciphertext_byte_fails_authentication() {
    let key = [0x11u8; 32];
    let plaintext = b"sensitive payload".to_vec();
    let mut ct = encrypt_chunk(&key, &plaintext);
    ct[0] ^= 0x01; // index 0 is within the body for non-empty plaintext
    let err = decrypt_chunk(&key, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn flipping_a_tag_byte_fails_authentication() {
    let key = [0x22u8; 32];
    let plaintext = b"sensitive payload".to_vec();
    let mut ct = encrypt_chunk(&key, &plaintext);
    let last = ct.len() - 1; // within the 16-byte tag
    ct[last] ^= 0x80;
    let err = decrypt_chunk(&key, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn wrong_key_fails_authentication() {
    let key = [0x33u8; 32];
    let wrong = [0x44u8; 32];
    let ct = encrypt_chunk(&key, b"hello");
    let err = decrypt_chunk(&wrong, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn truncated_ciphertext_fails() {
    let key = [0x55u8; 32];
    let ct = encrypt_chunk(&key, b"hello world");
    let truncated = &ct[..ct.len() - 4];
    let err = decrypt_chunk(&key, truncated).unwrap_err();
    assert_eq!(err, TamperError);
}
