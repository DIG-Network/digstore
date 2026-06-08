#[test]
fn derive_decryption_key_public_is_32_bytes_and_deterministic() {
    let canonical = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/file.txt";
    let k1 = digstore_crypto::derive_decryption_key(canonical, None);
    let k2 = digstore_crypto::derive_decryption_key(canonical, None);
    assert_eq!(k1.len(), 32);
    assert_eq!(k1, k2, "derivation must be deterministic for a given URN");
}
