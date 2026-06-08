use digstore_core::sha256;
use digstore_core::urn::Urn;
use digstore_core::Bytes32;

fn store_id() -> Bytes32 {
    Bytes32([0x11; 32])
}
fn root_hash() -> Bytes32 {
    Bytes32([0x22; 32])
}

#[test]
fn parse_full_urn() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}/path/to/file.txt");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.chain, "mainnet");
    assert_eq!(urn.store_id, store_id());
    assert_eq!(urn.root_hash, Some(root_hash()));
    assert_eq!(urn.resource_key.as_deref(), Some("path/to/file.txt"));
}

#[test]
fn parse_omitted_roothash_and_resource() {
    let sid = store_id().to_hex();
    let s = format!("urn:dig:testnet:{sid}");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.chain, "testnet");
    assert_eq!(urn.root_hash, None);
    assert_eq!(urn.resource_key, None);
}

#[test]
fn parse_resource_without_roothash() {
    let sid = store_id().to_hex();
    let s = format!("urn:dig:mainnet:{sid}/readme.md");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.root_hash, None);
    assert_eq!(urn.resource_key.as_deref(), Some("readme.md"));
}

#[test]
fn parse_roothash_without_resource() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.root_hash, Some(root_hash()));
    assert_eq!(urn.resource_key, None);
}

#[test]
fn canonical_roundtrips_parse() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}/a/b");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.canonical(), s);
    // Re-parsing the canonical form yields an equal URN.
    assert_eq!(Urn::parse(&urn.canonical()).unwrap(), urn);
}

#[test]
fn canonical_omits_absent_fields() {
    let sid = store_id().to_hex();
    let urn = Urn {
        chain: "mainnet".into(),
        store_id: store_id(),
        root_hash: None,
        resource_key: None,
    };
    assert_eq!(urn.canonical(), format!("urn:dig:mainnet:{sid}"));
}

#[test]
fn retrieval_key_is_sha256_of_canonical() {
    let sid = store_id().to_hex();
    let urn = Urn {
        chain: "mainnet".into(),
        store_id: store_id(),
        root_hash: None,
        resource_key: None,
    };
    let expected = sha256(format!("urn:dig:mainnet:{sid}").as_bytes());
    assert_eq!(urn.retrieval_key(), expected);
}

#[test]
fn parse_rejects_bad_scheme() {
    assert!(Urn::parse("urn:other:mainnet:00").is_err());
    assert!(Urn::parse("not-a-urn").is_err());
    assert!(Urn::parse("urn:dig:mainnet").is_err()); // missing store id
}

#[test]
fn parse_rejects_bad_store_id_hex() {
    assert!(Urn::parse("urn:dig:mainnet:zz").is_err());
}
