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

#[test]
fn commit_creates_generation_and_advances_history() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1_717_000_000)).unwrap();
    store.stage_file("index.html", &vec![0xABu8; 200_000]).unwrap();

    let root = store.commit().unwrap();

    let hist = store.root_history().unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].id, 0);
    assert_eq!(hist[0].root, root);
    assert_eq!(hist[0].timestamp, 1_717_000_000);

    let root_hex = root.to_hex();
    assert!(store.paths().generation_manifest(&root_hex).exists());
    assert!(store.paths().generation_chunks_dir(&root_hex).is_dir());

    assert!(
        digstore_store::StagingArea::open(store.paths().staging_file())
            .unwrap()
            .is_empty()
            .unwrap()
    );
}

#[test]
fn commit_refuses_empty_staging() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let err = store.commit().unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::EmptyStaging));
}

#[test]
fn commit_is_deterministic_for_fixed_input() {
    // Two independent stores with identical store_id, content, and clock must
    // produce the identical root hash (store-side determinism feeding §19.3).
    fn build() -> Bytes32 {
        let dir = tempdir().unwrap();
        let mut store = Store::init(config(dir.path()), FixedClock::new(42)).unwrap();
        store.stage_file("a.txt", b"deterministic content here").unwrap();
        store.stage_file("b.txt", &vec![7u8; 100_000]).unwrap();
        store.commit().unwrap()
    }
    assert_eq!(build(), build());
}

#[test]
fn commit_static_key_matches_client_url_retrieval_key() {
    // The manifest's static_key for a resource equals the retrieval key a client
    // computes from the canonical root-less URN (documented root-independence).
    use digstore_core::Urn;
    use digstore_store::GenerationManifest;

    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("index.html", b"hello").unwrap();
    let root = store.commit().unwrap();

    let manifest =
        GenerationManifest::read_from(store.paths().generation_manifest(&root.to_hex())).unwrap();
    let rec = manifest
        .key_table
        .iter()
        .find(|r| r.resource_key == "index.html")
        .unwrap();

    let client_urn = Urn {
        chain: "chia".to_string(),
        store_id: Bytes32([0x44u8; 32]),
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    assert_eq!(rec.static_key, client_urn.retrieval_key());
    assert_eq!(rec.generation, root);
}

fn count_all_chunk_files(generations_dir: &std::path::Path) -> usize {
    let mut n = 0;
    for gen in std::fs::read_dir(generations_dir).unwrap() {
        let chunks = gen.unwrap().path().join("chunks");
        if chunks.is_dir() {
            for e in std::fs::read_dir(&chunks).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_file()
                    && !e.file_name().to_string_lossy().ends_with(".tmp")
                {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn shared_chunk_is_stored_once_across_generations() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    // Generation 0: one big resource (forces several chunks).
    let payload = vec![0x5Au8; 300_000];
    store.stage_file("data.bin", &payload).unwrap();
    let root0 = store.commit().unwrap();

    // Generation 1: identical resource bytes -> identical chunks -> all dedup,
    // plus one brand-new chunk from a second resource.
    store.stage_file("data.bin", &payload).unwrap();
    store.stage_file("note.txt", b"a unique small note").unwrap();
    let root1 = store.commit().unwrap();

    assert_ne!(root0, root1, "different generations have different roots");

    use digstore_store::GenerationManifest;
    let m0 = GenerationManifest::read_from(store.paths().generation_manifest(&root0.to_hex())).unwrap();
    let m1 = GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
    let mut union = m0.chunk_hashes();
    union.extend(m1.chunk_hashes());

    let on_disk = count_all_chunk_files(&store.paths().generations_dir());
    assert_eq!(
        on_disk,
        union.len(),
        "each unique chunk stored exactly once across all generations"
    );
}

#[test]
fn resolve_chunk_reads_deduplicated_chunk_across_generations() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    let payload = vec![0x5Au8; 300_000];
    store.stage_file("data.bin", &payload).unwrap();
    let root0 = store.commit().unwrap();

    // Generation 1 re-stages identical bytes for data.bin (its chunks are
    // deduplicated to generation 0's chunk files -> sparse chunks/ dir under
    // generation 1) PLUS a new resource so the overall root differs from
    // generation 0 (identical content would otherwise yield an identical root).
    store.stage_file("data.bin", &payload).unwrap();
    store.stage_file("note.txt", b"a unique small note").unwrap();
    let root1 = store.commit().unwrap();
    assert_ne!(root0, root1, "generation 1 must have a distinct root");

    use digstore_store::GenerationManifest;
    let m1 = GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
    // The first chunk belongs to data.bin (staged first) and is shared with
    // generation 0, so it is deduplicated away from generation 1's dir.
    let shared = m1.chunks[0].hash;

    // The chunk file does NOT exist under generation 1 (deduplicated)...
    let gen1_local = store
        .paths()
        .chunk_file(&root1.to_hex(), &shared.to_hex());
    assert!(!gen1_local.exists(), "dedup leaves generation 1 chunks/ sparse");

    // ...but resolve_chunk finds it globally and returns the correct bytes.
    let bytes = store.resolve_chunk(shared).unwrap();
    assert_eq!(bytes.len(), m1.chunks[0].size as usize);
    assert_eq!(digstore_crypto::sha256(&bytes), shared);
}

#[test]
fn resolve_unknown_chunk_errors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("a.txt", b"x").unwrap();
    let _r = store.commit().unwrap();
    let bogus = Bytes32([0xCDu8; 32]);
    let err = store.resolve_chunk(bogus).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::ChunkNotFound(_)));
}
