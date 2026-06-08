#[test]
fn derive_decryption_key_public_is_32_bytes_and_deterministic() {
    let canonical = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/file.txt";
    let k1 = digstore_crypto::derive_decryption_key(canonical, None);
    let k2 = digstore_crypto::derive_decryption_key(canonical, None);
    assert_eq!(k1.len(), 32);
    assert_eq!(k1, k2, "derivation must be deterministic for a given URN");
}

#[test]
fn two_distinct_urns_yield_two_distinct_keys() {
    let a = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/a.txt";
    let b = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/b.txt";
    let ka = digstore_crypto::derive_decryption_key(a, None);
    let kb = digstore_crypto::derive_decryption_key(b, None);
    assert_ne!(ka, kb, "distinct URNs MUST derive distinct keys (fixed-nonce safety)");
}

#[test]
fn public_and_private_same_urn_yield_distinct_keys() {
    use digstore_core::SecretSalt;
    let u = "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a";
    let pub_k = digstore_crypto::derive_decryption_key(u, None);
    let priv_k = digstore_crypto::derive_decryption_key(u, Some(&SecretSalt([0x09; 32])));
    assert_ne!(pub_k, priv_k, "private store must not collide with public key for same URN");
}

#[test]
fn two_private_salts_same_urn_yield_distinct_keys() {
    use digstore_core::SecretSalt;
    let u = "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a";
    let k1 = digstore_crypto::derive_decryption_key(u, Some(&SecretSalt([0x01; 32])));
    let k2 = digstore_crypto::derive_decryption_key(u, Some(&SecretSalt([0x02; 32])));
    assert_ne!(k1, k2, "different SecretSalts must derive different keys");
}
