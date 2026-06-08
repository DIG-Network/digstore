use digstore_core::codec::{Decode, Encode};
use digstore_core::keytable::{KeyTableEntry, PathWalk};
use digstore_core::Bytes32;

#[test]
fn keytable_entry_roundtrip() {
    let e = KeyTableEntry {
        static_key: Bytes32([1; 32]),
        generation: Bytes32([2; 32]),
        chunk_indices: vec![0, 5, 9, 100],
        total_size: 4096,
    };
    let bytes = e.to_bytes();
    assert_eq!(KeyTableEntry::from_bytes(&bytes).unwrap(), e);
}

#[test]
fn keytable_entry_wire_layout() {
    let e = KeyTableEntry {
        static_key: Bytes32([0; 32]),
        generation: Bytes32([0; 32]),
        chunk_indices: vec![],
        total_size: 0,
    };
    let bytes = e.to_bytes();
    // 32 + 32 + 4(count=0) + 8(total_size) = 76 bytes
    assert_eq!(bytes.len(), 76);
}

#[test]
fn pathwalk_roundtrip_and_cursor() {
    let pw = PathWalk {
        resource_key: Bytes32([7; 32]),
        chunk_indices: vec![3, 4, 5],
        cursor: 1,
    };
    let bytes = pw.to_bytes();
    let decoded = PathWalk::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, pw);
    assert_eq!(decoded.cursor, 1);
}
