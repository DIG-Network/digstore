use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_store::{FixedClock, StagingArea, Store};
use std::io::Write;
use tempfile::tempdir;

fn config(dir: &std::path::Path) -> StoreConfig {
    StoreConfig {
        store_id: Bytes32([0x44u8; 32]),
        data_dir: dir.to_string_lossy().to_string(),
        max_size: 10_000_000,
        visibility: Visibility::Public,
    }
}

#[test]
fn stage_file_appends_to_staging() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("index.html", b"<html>hello</html>").unwrap();

    let staged = StagingArea::open(store.paths().staging_file())
        .unwrap()
        .records()
        .unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].resource_key, "index.html");
    assert_eq!(staged[0].content, b"<html>hello</html>");
}

#[test]
fn add_uses_relative_path_as_resource_key() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    let src_dir = tempdir().unwrap();
    let nested = src_dir.path().join("assets");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("logo.svg");
    let mut f = std::fs::File::create(&file).unwrap();
    f.write_all(b"<svg/>").unwrap();

    store.add(&file, src_dir.path()).unwrap();

    let staged = StagingArea::open(store.paths().staging_file())
        .unwrap()
        .records()
        .unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].resource_key, "assets/logo.svg");
    assert_eq!(staged[0].content, b"<svg/>");
}

#[test]
fn add_rejects_file_outside_base() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let base = tempdir().unwrap();
    let other = tempdir().unwrap();
    let file = other.path().join("x.txt");
    std::fs::write(&file, b"x").unwrap();
    let err = store.add(&file, base.path()).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::PathEscape(_)));
}
