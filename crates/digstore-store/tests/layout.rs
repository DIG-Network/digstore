use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_store::{FixedClock, Store};
use tempfile::tempdir;

fn config(dir: &std::path::Path) -> StoreConfig {
    StoreConfig {
        store_id: Bytes32([0x33u8; 32]),
        data_dir: dir.to_string_lossy().to_string(),
        max_size: 10_000_000,
        visibility: Visibility::Public,
    }
}

#[test]
fn init_creates_exact_layout() {
    let dir = tempdir().unwrap();
    let clock = FixedClock::new(1_717_000_000);
    let store = Store::init(config(dir.path()), clock).unwrap();

    let sid_hex = "33".repeat(32);
    assert!(dir.path().join("config.toml").exists(), "config.toml");
    assert!(
        dir.path().join(format!("{sid_hex}.staging.bin")).exists(),
        "staging"
    );
    assert!(dir.path().join("roots.log").exists(), "roots.log");
    assert!(dir.path().join("generations").is_dir(), "generations dir");
    assert!(dir.path().join("modules").is_dir(), "modules dir");

    assert_eq!(store.store_id(), Bytes32([0x33u8; 32]));
    assert!(store.root_history().unwrap().is_empty());
}

#[test]
fn init_refuses_to_clobber_existing_store() {
    let dir = tempdir().unwrap();
    Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let err = Store::init(config(dir.path()), FixedClock::new(1)).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::AlreadyExists(_)));
}

#[test]
fn open_reloads_an_existing_store() {
    let dir = tempdir().unwrap();
    Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let reopened = Store::open(dir.path(), FixedClock::new(99)).unwrap();
    assert_eq!(reopened.store_id(), Bytes32([0x33u8; 32]));
    assert!(matches!(reopened.config().visibility, Visibility::Public));
}

#[test]
fn open_missing_store_errors() {
    let dir = tempdir().unwrap();
    let err = Store::open(dir.path(), FixedClock::new(1)).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::NotFound(_)));
}
