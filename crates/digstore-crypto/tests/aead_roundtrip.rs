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
    assert_eq!(ct.len(), plaintext.len() + 16, "ct must carry a 16-byte GCM tag");
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
