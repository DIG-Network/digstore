use digstore_compiler::deterministic_filler;
use digstore_core::Bytes32;

#[test]
fn same_seed_inputs_yield_same_bytes() {
    let sid = Bytes32([1; 32]);
    let root = Bytes32([2; 32]);
    let a = deterministic_filler(&sid, &root, 100);
    let b = deterministic_filler(&sid, &root, 100);
    assert_eq!(a, b);
    assert_eq!(a.len(), 100);
}

#[test]
fn different_store_id_changes_stream() {
    let root = Bytes32([2; 32]);
    let a = deterministic_filler(&Bytes32([1; 32]), &root, 64);
    let b = deterministic_filler(&Bytes32([9; 32]), &root, 64);
    assert_ne!(a, b);
}

#[test]
fn different_roothash_changes_stream() {
    let sid = Bytes32([1; 32]);
    let a = deterministic_filler(&sid, &Bytes32([2; 32]), 64);
    let b = deterministic_filler(&sid, &Bytes32([3; 32]), 64);
    assert_ne!(a, b);
}

#[test]
fn prefix_property_first_bytes_match_longer_request() {
    let sid = Bytes32([7; 32]);
    let root = Bytes32([8; 32]);
    let short = deterministic_filler(&sid, &root, 16);
    let long = deterministic_filler(&sid, &root, 64);
    assert_eq!(short, &long[..16]);
}

#[test]
fn zero_length_is_empty() {
    let f = deterministic_filler(&Bytes32([0; 32]), &Bytes32([0; 32]), 0);
    assert!(f.is_empty());
}
