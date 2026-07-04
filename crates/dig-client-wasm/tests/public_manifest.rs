//! The `readPublicManifest` reader: decode a `.dig` data-section blob's normalized
//! public manifest to JSON, and tolerate its absence (older `.dig` / private store).

use dig_client_wasm::read_public_manifest;
use digstore_core::datasection::{encode_blob, encode_public_manifest, SectionId};
use digstore_core::{Bytes32, PublicManifest, PublicManifestEntry};

fn blob_with_manifest(pm: &PublicManifest) -> Vec<u8> {
    encode_blob(&[
        (SectionId::StoreId as u16, vec![1u8; 32]),
        (SectionId::PublicManifest as u16, encode_public_manifest(pm)),
    ])
}

#[test]
fn reads_embedded_manifest_as_json() {
    let pm = PublicManifest::new(vec![PublicManifestEntry {
        path: "index.html".into(),
        latest_root: Bytes32([0xab; 32]),
        generation_index: 3,
        sha256_latest: Bytes32([0xcd; 32]),
        version_count: 2,
    }]);
    let blob = blob_with_manifest(&pm);
    let json = read_public_manifest(&blob)
        .expect("valid blob")
        .expect("manifest present");
    assert!(json.contains("\"path\""));
    assert!(json.contains("\"index.html\""));
    assert!(json.contains("\"latest_root\""));
    assert!(json.contains(&"ab".repeat(32)));
    assert!(json.contains("\"generation_index\""));
    assert!(json.contains("\"sha256_latest\""));
    assert!(json.contains(&"cd".repeat(32)));
    assert!(json.contains("\"version_count\""));
    assert!(json.contains("\"schema_version\""));
}

#[test]
fn absent_manifest_is_null() {
    // A blob with no PublicManifest section (older .dig / private store) → None.
    let blob = encode_blob(&[(SectionId::StoreId as u16, vec![1u8; 32])]);
    assert!(read_public_manifest(&blob).expect("valid blob").is_none());
}
// NOTE: the malformed-blob error path (a thrown `JsError`) is only exercisable
// under wasm — constructing a `JsError` panics on the native test target — and is
// covered by `digstore_core::datasection::DataView::parse`'s own reject tests.
