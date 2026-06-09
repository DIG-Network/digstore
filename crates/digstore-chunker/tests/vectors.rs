use digstore_chunker::GEAR_TABLE;

#[test]
fn gear_table_has_256_entries() {
    assert_eq!(GEAR_TABLE.len(), 256);
}

#[test]
fn gear_table_is_nontrivial() {
    // Not the all-zero placeholder from the scaffold.
    assert!(
        GEAR_TABLE.iter().any(|&x| x != 0),
        "gear table must not be all zero"
    );
    // High-quality table: every entry distinct so no two bytes alias.
    let mut seen = std::collections::HashSet::new();
    for &v in GEAR_TABLE.iter() {
        assert!(
            seen.insert(v),
            "gear table entries must be distinct, found dup {v:#018x}"
        );
    }
}

#[test]
fn gear_table_pinned_guards_are_present() {
    // Pin two values so the table can never silently change (determinism guard).
    assert_eq!(GEAR_TABLE[0], 0x3b5c_9f8e_2d71_a046);
    assert_eq!(GEAR_TABLE[255], 0x9e1d_4a7c_60b3_82f5);
}

use digstore_chunker::{chunk_slice, default_config};

/// Deterministic pseudo-random input generator (xorshift64; fully reproducible,
/// no external RNG).
fn fixed_input(n: usize) -> Vec<u8> {
    let mut state: u64 = 0x0123_4567_89ab_cdef;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

#[test]
#[ignore = "capture-only: run with --ignored --nocapture to print golden values"]
fn capture_golden_boundary_values() {
    let data = fixed_input(200 * 1024);
    let cfg = default_config();
    let chunks = chunk_slice(&data, &cfg);
    let lengths: Vec<usize> = chunks.iter().map(|c| c.data.len()).collect();
    eprintln!("GOLDEN_LENGTHS={lengths:?}");
    eprintln!("GOLDEN_HASH0={}", chunks[0].hash.to_hex());
    eprintln!("GOLDEN_CHUNK_COUNT={}", chunks.len());
}

#[test]
fn golden_boundaries_are_stable() {
    let data = fixed_input(200 * 1024);
    let cfg = default_config();
    let chunks = chunk_slice(&data, &cfg);

    // Reconstruction sanity.
    let total: usize = chunks.iter().map(|c| c.data.len()).sum();
    assert_eq!(total, data.len());

    // FROZEN boundary sequence (captured from Step 8.2 GOLDEN_LENGTHS).
    let lengths: Vec<usize> = chunks.iter().map(|c| c.data.len()).collect();
    let expected_lengths: Vec<usize> = vec![192069, 12731];
    assert_eq!(lengths, expected_lengths, "chunk boundary sequence changed");

    // FROZEN first-chunk content address (captured from Step 8.2 GOLDEN_HASH0).
    assert_eq!(
        chunks[0].hash.to_hex(),
        "99f162103cb5b0cff95dcfee4ba9b343e1ea865e0ab9416845898bb57ad1e105"
    );
}

/// Count shared trailing chunk content-addresses between two chunkings.
fn shared_trailing_hashes(a: &[digstore_chunker::Chunk], b: &[digstore_chunker::Chunk]) -> usize {
    let mut shared = 0usize;
    while shared < a.len()
        && shared < b.len()
        && a[a.len() - 1 - shared].hash == b[b.len() - 1 - shared].hash
    {
        shared += 1;
    }
    shared
}

#[test]
#[ignore = "capture-only: run with --ignored --nocapture to print dedup-locality value"]
fn capture_dedup_locality_value() {
    let cfg = default_config();
    let body = fixed_input(300 * 1024);
    let mut modified = vec![0xABu8; 1000]; // 1000-byte prepend
    modified.extend_from_slice(&body);

    let original_chunks = chunk_slice(&body, &cfg);
    let modified_chunks = chunk_slice(&modified, &cfg);
    let shared = shared_trailing_hashes(&original_chunks, &modified_chunks);

    eprintln!("DEDUP_ORIGINAL_CHUNKS={}", original_chunks.len());
    eprintln!("DEDUP_MODIFIED_CHUNKS={}", modified_chunks.len());
    eprintln!("DEDUP_SHARED_TRAILING={shared}");
}

#[test]
fn front_insert_preserves_trailing_chunks() {
    let cfg = default_config();
    let body = fixed_input(300 * 1024);
    let mut modified = vec![0xABu8; 1000];
    modified.extend_from_slice(&body);

    let original_chunks = chunk_slice(&body, &cfg);
    let modified_chunks = chunk_slice(&modified, &cfg);
    let shared = shared_trailing_hashes(&original_chunks, &modified_chunks);

    // FROZEN observed re-synchronization (captured DEDUP_SHARED_TRAILING from Step 9.2).
    // A fixed-block chunker would share ZERO trailing chunks after a front insert;
    // CDC re-synchronizes and shares this many (paper §3 dedup heritage).
    let expected_shared: usize = 1;
    assert_eq!(shared, expected_shared, "dedup re-sync count changed");
    assert!(
        shared >= 1,
        "CDC must share at least one trailing chunk after front insert"
    );
}

use digstore_chunker::Chunker;

#[test]
fn chunker_struct_roundtrip_via_public_api() {
    let cfg = default_config();
    let chunker = Chunker::new(cfg);
    assert_eq!(chunker.config().target_size, 64 * 1024);

    let data = fixed_input(300 * 1024);
    let chunks = chunker.chunk_slice(&data);

    // Reconstruct and confirm every content address is a 64-hex-char SHA-256.
    let mut rebuilt = Vec::new();
    for c in &chunks {
        rebuilt.extend_from_slice(&c.data);
        assert_eq!(c.hash.to_hex().len(), 64);
    }
    assert_eq!(rebuilt, data);
    assert!(
        chunks.len() > 1,
        "300 KiB under 64 KiB target should yield multiple chunks"
    );
}
