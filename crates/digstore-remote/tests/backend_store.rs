mod test_helpers;
use test_helpers::*;

use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_remote::{backend::RemoteBackend, PushMode, PushOutcome, RemoteError, StoreBackend};

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
        label: None,
        description: None,
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
fn accept_push_with_a_stale_parent_is_rejected_directly() {
    // #131 trust-boundary defense-in-depth: accept_push is a public trait method
    // callable WITHOUT the HTTP handler's parent==head check. A direct caller
    // declaring a stale/wrong parent must NOT be able to append a non-fast-forward
    // generation — accept_push re-asserts the fast-forward itself.
    let tmp = unique_tmp("ff");
    let store_id = b32(0x44);
    let config = config_at(&tmp, store_id);
    let backend = StoreBackend::initialize_for_test(config, b48(7), vec![0u8; 64], b32(0x10), None)
        .expect("init store backend");

    // Served head is genesis 0x10. A push declaring a WRONG parent (0xEE) must fail
    // closed with NonFastForward, even called directly on the backend.
    let stale = backend.accept_push(
        &store_id,
        &b32(0xEE),
        &b32(0x20),
        &[1u8; 64],
        None,
        PushMode::Advance,
    );
    assert!(
        matches!(stale, Err(RemoteError::NonFastForward)),
        "a direct accept_push with a stale parent must be rejected: {stale:?}"
    );

    // The head must be unchanged (the stale push did not advance it).
    assert_eq!(
        backend.head_state(&store_id).unwrap().served_root,
        b32(0x10)
    );

    // The same push declaring the CORRECT parent (the served head 0x10) advances.
    let ok = backend.accept_push(
        &store_id,
        &b32(0x10),
        &b32(0x20),
        &[1u8; 64],
        None,
        PushMode::Advance,
    );
    assert!(
        matches!(ok, Ok(PushOutcome::Advanced)),
        "a fast-forward push from the served head advances: {ok:?}"
    );
    assert_eq!(
        backend.head_state(&store_id).unwrap().served_root,
        b32(0x20)
    );
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
