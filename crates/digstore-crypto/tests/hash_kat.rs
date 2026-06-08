use digstore_crypto::sha256;

#[test]
fn sha256_known_answer_abc() {
    // FIPS 180-2 test vector for "abc".
    let got = sha256(b"abc");
    let expected =
        hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            .unwrap();
    assert_eq!(&got.0[..], &expected[..]);
}

#[test]
fn sha256_known_answer_empty() {
    let got = sha256(b"");
    let expected =
        hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap();
    assert_eq!(&got.0[..], &expected[..]);
}

#[test]
fn crate_advertises_its_version() {
    assert_eq!(digstore_crypto::CRYPTO_VERSION, 1);
}
