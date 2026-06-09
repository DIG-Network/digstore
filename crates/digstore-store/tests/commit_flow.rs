use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_store::{FixedClock, StagingArea, Store};
use std::io::Write;
use tempfile::tempdir;

/// The canonical chunker config `Store::commit` uses (mirror for tests).
fn test_chunker() -> digstore_core::ChunkerConfig {
    digstore_core::ChunkerConfig {
        min_size: 16 * 1024,
        target_size: 64 * 1024,
        max_size: 256 * 1024,
        mask: (1u64 << 16) - 1,
    }
}

/// Independently recompute the D5 per-resource ciphertext merkle leaves for a
/// set of staged `(resource_key, plaintext)` pairs under `store_id`, exactly as
/// the compiler does: leaf = SHA-256(concat_output(ordered chunk ciphertexts)),
/// resources ascending by static_key.
fn expected_resource_leaves(
    store_id: Bytes32,
    salt: Option<&digstore_core::SecretSalt>,
    staged: &[(&str, &[u8])],
) -> Vec<Bytes32> {
    use digstore_chunker::chunk_slice;
    use digstore_core::serving::concat_output;
    use digstore_core::Urn;

    let mut keyed: Vec<([u8; 32], Bytes32)> = staged
        .iter()
        .map(|(rk, content)| {
            let urn = Urn {
                chain: "chia".to_string(),
                store_id,
                root_hash: None,
                resource_key: Some(rk.to_string()),
            };
            let aes_key = digstore_crypto::derive_decryption_key(&urn.canonical(), salt);
            let chunks = chunk_slice(content, &test_chunker());
            let cts: Vec<Vec<u8>> = chunks
                .iter()
                .map(|c| digstore_crypto::encrypt_chunk(&aes_key, &c.data))
                .collect();
            let slices: Vec<&[u8]> = cts.iter().map(|c| c.as_slice()).collect();
            let blob = concat_output(&slices);
            (urn.retrieval_key().0, digstore_crypto::sha256(&blob))
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, leaf)| leaf).collect()
}

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
    store
        .stage_file("index.html", b"<html>hello</html>")
        .unwrap();

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
    store
        .stage_file("index.html", &vec![0xABu8; 200_000])
        .unwrap();

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
fn commit_generation_root_equals_recomputed_tree_root() {
    // §9.4 invariant (D5 model): the PERSISTED GenerationState.root equals the
    // Merkle tree root freshly recomputed from the persisted ON-DISK state — one
    // leaf PER RESOURCE = SHA-256(concat_output(ordered chunk ciphertexts)),
    // resources ascending by static_key. We re-derive the root purely from the
    // persisted manifest + persisted ciphertext chunk bodies (resolved by hash),
    // independent of the value `commit()` returned.
    use digstore_core::merkle::MerkleTree;
    use digstore_core::serving::concat_output;
    use digstore_core::Bytes32 as CoreBytes32;
    use digstore_store::GenerationManifest;
    use std::collections::BTreeMap;

    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    // Force several chunks across two resources so the tree has interior nodes.
    store.stage_file("data.bin", &vec![0x5Au8; 300_000]).unwrap();
    store.stage_file("note.txt", b"a small note").unwrap();
    let _returned = store.commit().unwrap();

    // Persisted head root (GenerationState.root), read back from history.
    let hist = store.root_history().unwrap();
    let persisted_state_root = hist.last().unwrap().root;

    // Recompute the per-resource ciphertext leaves from on-disk state.
    let manifest = GenerationManifest::read_from(
        store.paths().generation_manifest(&persisted_state_root.to_hex()),
    )
    .unwrap();
    // index -> ciphertext-hash, so chunk_indices can locate the stored bytes.
    let by_index: BTreeMap<u32, CoreBytes32> =
        manifest.chunks.iter().map(|c| (c.index, c.hash)).collect();

    // One leaf per resource, ascending by static_key (raw 32 bytes).
    let mut keyed: Vec<([u8; 32], CoreBytes32)> = manifest
        .key_table
        .iter()
        .map(|kt| {
            let cts: Vec<Vec<u8>> = kt
                .chunk_indices
                .iter()
                .map(|i| {
                    let hash = by_index[i];
                    store.resolve_chunk(hash).unwrap() // persisted ciphertext bytes
                })
                .collect();
            let slices: Vec<&[u8]> = cts.iter().map(|b| b.as_slice()).collect();
            let blob = concat_output(&slices);
            (kt.static_key.0, digstore_crypto::sha256(&blob))
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let leaves: Vec<CoreBytes32> = keyed.into_iter().map(|(_, leaf)| leaf).collect();
    let recomputed = MerkleTree::from_leaves(leaves).root();

    assert_eq!(
        persisted_state_root, recomputed,
        "persisted GenerationState.root must equal the freshly recomputed per-resource tree root"
    );
    // The manifest's own root field must agree too (state == manifest == tree).
    assert_eq!(manifest.root, recomputed);
}

#[test]
fn commit_state_root_equals_per_resource_ciphertext_tree_root() {
    // D5 / §9.4: the persisted GenerationState.root MUST equal the merkle tree
    // built over PER-RESOURCE ciphertext leaves (leaf = SHA-256(concat_output(
    // ordered chunk ciphertexts)), resources ascending by static_key) — the
    // exact same leaves the compiler injects as MerkleNodes and over which it
    // sets CurrentRoot. This makes GenerationState.root == compiler CurrentRoot
    // for the same inputs.
    use digstore_core::merkle::MerkleTree;

    let dir = tempdir().unwrap();
    let store_id = Bytes32([0x44u8; 32]);
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    // Two resources, one large enough to force multiple chunks.
    let big = vec![0x5Au8; 300_000];
    store.stage_file("data.bin", &big).unwrap();
    store.stage_file("note.txt", b"a small note").unwrap();
    let returned = store.commit().unwrap();

    // Persisted head root (GenerationState.root), read back from history.
    let hist = store.root_history().unwrap();
    let persisted_state_root = hist.last().unwrap().root;
    assert_eq!(persisted_state_root, returned);

    // Independently recompute the per-resource ciphertext leaves + tree root.
    let leaves = expected_resource_leaves(
        store_id,
        None, // public store -> no secret salt
        &[("data.bin", &big), ("note.txt", b"a small note")],
    );
    let expected_root = MerkleTree::from_leaves(leaves).root();

    assert_eq!(
        persisted_state_root, expected_root,
        "GenerationState.root must equal the per-resource ciphertext tree root"
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
        store
            .stage_file("a.txt", b"deterministic content here")
            .unwrap();
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
    store
        .stage_file("note.txt", b"a unique small note")
        .unwrap();
    let root1 = store.commit().unwrap();

    assert_ne!(root0, root1, "different generations have different roots");

    use digstore_store::GenerationManifest;
    let m0 =
        GenerationManifest::read_from(store.paths().generation_manifest(&root0.to_hex())).unwrap();
    let m1 =
        GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
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
    store
        .stage_file("note.txt", b"a unique small note")
        .unwrap();
    let root1 = store.commit().unwrap();
    assert_ne!(root0, root1, "generation 1 must have a distinct root");

    use digstore_store::GenerationManifest;
    let m1 =
        GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
    // The first chunk belongs to data.bin (staged first) and is shared with
    // generation 0, so it is deduplicated away from generation 1's dir.
    let shared = m1.chunks[0].hash;

    // The chunk file does NOT exist under generation 1 (deduplicated)...
    let gen1_local = store.paths().chunk_file(&root1.to_hex(), &shared.to_hex());
    assert!(
        !gen1_local.exists(),
        "dedup leaves generation 1 chunks/ sparse"
    );

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

#[test]
fn log_lists_generations_in_order() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store.stage_file("a.txt", b"first").unwrap();
    let _r0 = store.commit().unwrap();
    store.stage_file("b.txt", b"second").unwrap();
    let _r1 = store.commit().unwrap();

    let log = store.log().unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, 0);
    assert_eq!(log[1].id, 1);
}

#[test]
fn diff_between_two_generations_reports_added_key() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store
        .stage_file("a.txt", b"alpha content here padded out")
        .unwrap();
    let r0 = store.commit().unwrap();
    store
        .stage_file("a.txt", b"alpha content here padded out")
        .unwrap();
    store
        .stage_file("b.txt", b"a new resource entirely")
        .unwrap();
    let r1 = store.commit().unwrap();

    let d = store.diff(r0, r1).unwrap();
    assert_eq!(d.keys_added, vec!["b.txt".to_string()]);
    assert!(d.keys_removed.is_empty());
}

#[test]
fn diff_unknown_root_errors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store.stage_file("a.txt", b"x").unwrap();
    let r0 = store.commit().unwrap();
    let bogus = Bytes32([0xEEu8; 32]);
    let err = store.diff(r0, bogus).unwrap_err();
    assert!(matches!(
        err,
        digstore_store::StoreError::GenerationNotFound(_)
    ));
}

#[test]
fn current_root_and_history_accessors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    assert!(store.current_root().unwrap().is_none());

    store.stage_file("a.txt", b"one").unwrap();
    let r0 = store.commit().unwrap();
    store.stage_file("b.txt", b"two").unwrap();
    let r1 = store.commit().unwrap();

    assert_eq!(store.current_root().unwrap(), Some(r1));
    assert_eq!(store.roothash_history().unwrap(), vec![r0, r1]);

    let sid_hex = "44".repeat(32);
    let expected = dir
        .path()
        .join("modules")
        .join(format!("{sid_hex}-{}.wasm", r1.to_hex()));
    assert_eq!(store.module_path(r1), expected);
}
