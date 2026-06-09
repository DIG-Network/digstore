#[allow(unused_imports)]
use digstore_chunker::{chunk_slice, chunk_stream, Chunk};
use proptest::prelude::*;

fn small_cfg() -> digstore_core::ChunkerConfig {
    digstore_core::ChunkerConfig {
        min_size: 64,
        target_size: 256,
        max_size: 1024,
        mask: 0xFF,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Determinism: chunking the same input twice yields identical chunks.
    #[test]
    fn determinism_same_input_same_chunks(data in proptest::collection::vec(any::<u8>(), 0..50_000)) {
        let cfg = small_cfg();
        let a = chunk_slice(&data, &cfg);
        let b = chunk_slice(&data, &cfg);
        prop_assert_eq!(a, b);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Concatenated chunks reconstruct the input; offsets contiguous; interior
    /// chunks obey size bounds.
    #[test]
    fn reconstruct_offsets_and_bounds(data in proptest::collection::vec(any::<u8>(), 0..50_000)) {
        let cfg = small_cfg();
        let chunks = chunk_slice(&data, &cfg);

        // Reconstruction.
        let mut rebuilt = Vec::with_capacity(data.len());
        for c in &chunks {
            rebuilt.extend_from_slice(&c.data);
        }
        prop_assert_eq!(&rebuilt, &data);

        // Offsets contiguous from 0.
        let mut off = 0usize;
        for c in &chunks {
            prop_assert_eq!(c.offset, off);
            off += c.data.len();
        }

        // Size bounds: all but the last chunk in [min, max]; last in [1, max].
        if chunks.len() > 1 {
            for c in &chunks[..chunks.len() - 1] {
                prop_assert!(c.len() >= cfg.min_size);
                prop_assert!(c.len() <= cfg.max_size);
            }
        }
        if let Some(last) = chunks.last() {
            prop_assert!(!last.is_empty());
            prop_assert!(last.len() <= cfg.max_size);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Non-flaky dedup observation: chunk both `body` and `prefix ++ body`; the
    /// shared trailing-chunk count is at most the number of chunks in either, and
    /// reconstruction holds for both. We do NOT assert shared >= 1 here (CDC
    /// re-sync is probabilistic over random data; the guaranteed case is the
    /// frozen vector in tests/vectors.rs). This only proves no panic / no
    /// inconsistency in the locality path.
    #[test]
    fn front_insert_locality_is_consistent(
        prefix in proptest::collection::vec(any::<u8>(), 1..2000),
        body in proptest::collection::vec(any::<u8>(), 5_000..40_000),
    ) {
        let cfg = small_cfg();
        let mut modified = prefix.clone();
        modified.extend_from_slice(&body);

        let oc = chunk_slice(&body, &cfg);
        let mc = chunk_slice(&modified, &cfg);

        // Both reconstruct.
        let oj: Vec<u8> = oc.iter().flat_map(|c| c.data.clone()).collect();
        prop_assert_eq!(&oj, &body);
        let mj: Vec<u8> = mc.iter().flat_map(|c| c.data.clone()).collect();
        prop_assert_eq!(&mj, &modified);

        // Shared trailing count is well-formed (<= min of the two chunk counts).
        let mut shared = 0usize;
        while shared < oc.len()
            && shared < mc.len()
            && oc[oc.len() - 1 - shared].hash == mc[mc.len() - 1 - shared].hash
        {
            shared += 1;
        }
        prop_assert!(shared <= oc.len().min(mc.len()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// chunk_stream over a Cursor equals chunk_slice for the same bytes.
    #[test]
    fn stream_equals_slice_property(data in proptest::collection::vec(any::<u8>(), 0..30_000)) {
        let cfg = small_cfg();
        let want = chunk_slice(&data, &cfg);
        let got = chunk_stream(std::io::Cursor::new(&data), &cfg).unwrap();
        prop_assert_eq!(got, want);
    }
}
