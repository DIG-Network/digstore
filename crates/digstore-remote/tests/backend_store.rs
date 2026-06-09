mod test_helpers;
use test_helpers::*;

use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_remote::{backend::RemoteBackend, StoreBackend};

fn unique_tmp(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "digstore-remote-test-{}-{}-{}",
        tag,
        std::process::id(),
        nanos
    ))
}

fn config_at(tmp: &std::path::Path, store_id: Bytes32) -> StoreConfig {
    std::fs::create_dir_all(tmp).unwrap();
    StoreConfig {
        store_id,
        data_dir: tmp.to_string_lossy().to_string(),
        max_size: 16 * 1024 * 1024,
        visibility: Visibility::Public,
    }
}

#[test]
fn head_state_matches_store_genesis() {
    let tmp = unique_tmp("head");
    let store_id = b32(0x42);
    let config = config_at(&tmp, store_id);
    // build a store with one generation + module via the production helper.
    let backend =
        StoreBackend::initialize_for_test(config, b48(7), vec![0u8; 128], b32(0x10), None)
            .expect("init store backend");
    let hs = backend.head_state(&store_id).unwrap();
    assert_eq!(hs.served_root, b32(0x10));
    assert_eq!(hs.served_size, 128);
    assert_eq!(hs.public_key, b48(7));
}

#[test]
fn serve_content_miss_returns_decoy_never_error() {
    let tmp = unique_tmp("decoy");
    let store_id = b32(0x43);
    let config = config_at(&tmp, store_id);
    let backend = StoreBackend::initialize_for_test(config, b48(7), vec![0u8; 64], b32(0x10), None)
        .expect("init store backend");
    let (ct, _proof, root) = backend
        .serve_content(&store_id, &b32(0x99), &b32(0x10), None)
        .expect("content miss must be Ok with a decoy, never an error (§14.2)");
    assert!(!ct.is_empty(), "decoy has real-looking bytes");
    assert_eq!(root, b32(0x10));
}
