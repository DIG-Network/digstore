use digstore_core::codec::{Decode, Encode};
use digstore_core::manifest::{Author, MetadataManifest};
use std::collections::BTreeMap;

#[test]
fn manifest_roundtrip_minimal() {
    let m = MetadataManifest {
        schema_version: 1,
        name: "my-store".into(),
        version: None,
        description: None,
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: BTreeMap::new(),
        custom: BTreeMap::new(),
    };
    let bytes = m.to_bytes();
    assert_eq!(MetadataManifest::from_bytes(&bytes).unwrap(), m);
}

#[test]
fn manifest_roundtrip_full() {
    let mut links = BTreeMap::new();
    links.insert("docs".to_string(), "https://example.com".to_string());
    let mut custom = BTreeMap::new();
    custom.insert("rating".to_string(), serde_json::json!(5));
    custom.insert("verified".to_string(), serde_json::json!(true));
    let m = MetadataManifest {
        schema_version: 2,
        name: "pkg".into(),
        version: Some("1.0.0".into()),
        description: Some("a package".into()),
        authors: vec![Author {
            name: "Alice".into(),
            handle: Some("@alice".into()),
            contact: None,
        }],
        license: Some("MIT".into()),
        homepage: Some("https://home".into()),
        repository: Some("https://repo".into()),
        keywords: vec!["a".into(), "b".into()],
        categories: vec!["tools".into()],
        icon: Some("icon.png".into()),
        content_type: Some("application/octet-stream".into()),
        links,
        custom,
    };
    let bytes = m.to_bytes();
    assert_eq!(MetadataManifest::from_bytes(&bytes).unwrap(), m);
}

#[test]
fn author_roundtrip() {
    let a = Author {
        name: "Bob".into(),
        handle: None,
        contact: Some("bob@x.io".into()),
    };
    let bytes = a.to_bytes();
    assert_eq!(Author::from_bytes(&bytes).unwrap(), a);
}
