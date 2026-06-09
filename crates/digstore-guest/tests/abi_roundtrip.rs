use digstore_core::abi::{is_error, pack_ptr_len, unpack_ptr_len};
use digstore_guest::packing::{guest_pack, guest_unpack};

#[test]
fn guest_pack_matches_core_pack() {
    for &(p, l) in &[
        (0u32, 0u32),
        (1, 2),
        (0x1234_5678, 0x0000_00FF),
        (u32::MAX, 16),
    ] {
        assert_eq!(
            guest_pack(p, l),
            pack_ptr_len(p, l),
            "pack must match core for {p},{l}"
        );
    }
}

#[test]
fn guest_unpack_matches_core_unpack() {
    let packed = pack_ptr_len(0xDEAD_BEEF, 1024);
    assert_eq!(guest_unpack(packed), unpack_ptr_len(packed));
    assert_eq!(guest_unpack(packed), (0xDEAD_BEEFu32, 1024u32));
}

#[test]
fn error_sentinel_round_trips() {
    // len==0 && (ptr as i32) < 0 => error per core::abi::is_error
    let err = pack_ptr_len(0xFFFF_FFFF, 0);
    assert!(
        is_error(err),
        "high-bit ptr with zero len is an error sentinel"
    );
    let ok = pack_ptr_len(16, 32);
    assert!(!is_error(ok));
}

use digstore_core::Bytes32;
use digstore_guest::request::{ContentRequest, ProofRequest, ValidityWindow};

#[test]
fn content_request_round_trips() {
    let req = ContentRequest {
        retrieval_key: Bytes32([7u8; 32]),
        root_hash: Some(Bytes32([9u8; 32])),
        range: Some((10, 200)),
        jwt: Some(b"header.payload.sig".to_vec()),
        window: Some(ValidityWindow {
            not_before: 100,
            not_after: 999,
        }),
    };
    let bytes = req.encode();
    let (decoded, consumed) = ContentRequest::decode(&bytes).expect("decode");
    assert_eq!(decoded, req);
    assert_eq!(consumed, bytes.len(), "decode must consume all bytes");
}

#[test]
fn content_request_minimal_round_trips() {
    let req = ContentRequest {
        retrieval_key: Bytes32([1u8; 32]),
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    let bytes = req.encode();
    let (decoded, _) = ContentRequest::decode(&bytes).expect("decode");
    assert_eq!(decoded, req);
}

#[test]
fn proof_request_round_trips() {
    let req = ProofRequest {
        retrieval_key: Bytes32([3u8; 32]),
        root_hash: Some(Bytes32([4u8; 32])),
        client_nonce: [5u8; 32],
    };
    let bytes = req.encode();
    let (decoded, _) = ProofRequest::decode(&bytes).expect("decode");
    assert_eq!(decoded, req);
}

#[test]
fn content_request_rejects_truncated() {
    assert!(ContentRequest::decode(&[0u8; 4]).is_err());
}
