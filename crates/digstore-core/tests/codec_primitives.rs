use digstore_core::codec::{Decode, Encode};

#[test]
fn u8_fixture() {
    assert_eq!(0x07u8.to_bytes(), vec![0x07]);
    assert_eq!(u8::from_bytes(&[0xFF]).unwrap(), 0xFF);
}

#[test]
fn u16_be_fixture() {
    assert_eq!(0x0102u16.to_bytes(), vec![0x01, 0x02]);
    assert_eq!(u16::from_bytes(&[0xAB, 0xCD]).unwrap(), 0xABCD);
}

#[test]
fn u32_be_fixture() {
    assert_eq!(0x01020304u32.to_bytes(), vec![0x01, 0x02, 0x03, 0x04]);
    assert_eq!(u32::from_bytes(&[0x00, 0x00, 0x01, 0x00]).unwrap(), 256);
}

#[test]
fn u64_be_fixture() {
    assert_eq!(
        0x0102030405060708u64.to_bytes(),
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    );
}

#[test]
fn option_none_is_zero_tag() {
    let v: Option<u32> = None;
    assert_eq!(v.to_bytes(), vec![0x00]);
}

#[test]
fn option_some_is_one_tag_then_value() {
    let v: Option<u32> = Some(0x01020304);
    assert_eq!(v.to_bytes(), vec![0x01, 0x01, 0x02, 0x03, 0x04]);
    assert_eq!(
        Option::<u32>::from_bytes(&[0x01, 0x00, 0x00, 0x00, 0x09]).unwrap(),
        Some(9)
    );
    assert_eq!(Option::<u32>::from_bytes(&[0x00]).unwrap(), None);
}

#[test]
fn option_invalid_tag_rejected() {
    assert!(Option::<u32>::from_bytes(&[0x02]).is_err());
}

#[test]
fn vec_count_prefixed_be() {
    let v: Vec<u16> = vec![0x0102, 0x0304];
    // 4-byte BE count (2) then two u16.
    assert_eq!(v.to_bytes(), vec![0, 0, 0, 2, 0x01, 0x02, 0x03, 0x04]);
    assert_eq!(
        Vec::<u16>::from_bytes(&[0, 0, 0, 0]).unwrap(),
        Vec::<u16>::new()
    );
}

#[test]
fn string_len_prefixed_utf8() {
    let s = String::from("dig");
    assert_eq!(s.to_bytes(), vec![0, 0, 0, 3, b'd', b'i', b'g']);
    assert_eq!(String::from_bytes(&[0, 0, 0, 0]).unwrap(), String::new());
}

#[test]
fn fixed_array_raw_no_prefix() {
    let a: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    assert_eq!(a.to_bytes(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(<[u8; 4]>::from_bytes(&[1, 2, 3, 4]).unwrap(), [1, 2, 3, 4]);
}

#[test]
fn eof_is_error() {
    assert!(u32::from_bytes(&[0x00, 0x01]).is_err());
}
