use digstore_core::bytes::{Bytes32, Bytes48, Bytes96};
use digstore_core::codec::{Decode, Encode};
use digstore_core::sha256;

#[test]
fn bytes32_hex_roundtrip() {
    let b = Bytes32([0xAB; 32]);
    let hex = b.to_hex();
    assert_eq!(hex.len(), 64);
    assert_eq!(Bytes32::from_hex(&hex).unwrap(), b);
}

#[test]
fn bytes32_from_hex_rejects_wrong_length() {
    assert!(Bytes32::from_hex("abcd").is_err());
}

#[test]
fn bytes32_codec_is_raw_32_bytes() {
    let b = Bytes32([7u8; 32]);
    let enc = b.to_bytes();
    assert_eq!(enc.len(), 32);
    assert_eq!(enc, vec![7u8; 32]);
    assert_eq!(Bytes32::from_bytes(&enc).unwrap(), b);
}

#[test]
fn bytes48_and_96_codec_lengths() {
    assert_eq!(Bytes48([1u8; 48]).to_bytes().len(), 48);
    assert_eq!(Bytes96([2u8; 96]).to_bytes().len(), 96);
    assert_eq!(Bytes48::from_bytes(&[3u8; 48]).unwrap(), Bytes48([3u8; 48]));
    assert_eq!(Bytes96::from_bytes(&[4u8; 96]).unwrap(), Bytes96([4u8; 96]));
}

#[test]
fn sha256_known_answer_empty() {
    let out = sha256(b"");
    assert_eq!(
        out.to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_known_answer_abc() {
    let out = sha256(b"abc");
    assert_eq!(
        out.to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
