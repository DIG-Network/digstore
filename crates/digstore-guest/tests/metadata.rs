mod fixtures;
use digstore_core::Bytes32;
use digstore_guest::datasection::DataSection;
use digstore_guest::metadata::{current_roothash, metadata_bytes, public_key, store_id};

#[test]
fn store_id_and_root_come_from_section() {
    let blob = fixtures::build_minimal_section([0x5A; 32], [0x6B; 32], &[]);
    let ds = DataSection::parse(&blob).unwrap();
    assert_eq!(store_id(&ds), Bytes32([0x5A; 32]));
    assert_eq!(current_roothash(&ds), Bytes32([0x6B; 32]));
}

#[test]
fn metadata_is_returned_verbatim_and_ungated() {
    // get_metadata returns the plaintext manifest section as-is, with no gate.
    let manifest = br#"{"schema_version":1,"name":"demo"}"#;
    let blob = fixtures::section_with_metadata([1; 32], [2; 32], manifest);
    let ds = DataSection::parse(&blob).unwrap();
    assert_eq!(metadata_bytes(&ds), manifest.to_vec());
}

#[test]
fn public_key_is_48_bytes() {
    let blob = fixtures::section_with_pubkey([1; 32], [2; 32], &[0xCD; 48]);
    let ds = DataSection::parse(&blob).unwrap();
    let pk = public_key(&ds);
    assert_eq!(pk.0.len(), 48);
    assert_eq!(pk.0[0], 0xCD);
}
