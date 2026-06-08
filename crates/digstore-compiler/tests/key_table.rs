mod common;

use digstore_compiler::build_chunk_index_and_key_table;
use digstore_core::Bytes32;

#[test]
fn fixtures_build_and_expose_two_generations() {
    let gens = common::sample_generations();
    assert_eq!(gens.len(), 2);
    let rk = common::resource_key("about.html");
    assert!(gens
        .iter()
        .any(|g| g.resources.iter().any(|r| r.resource_key == rk)));
}

#[test]
fn entries_map_resource_to_ordered_global_chunk_indices() {
    let gens = common::sample_generations();
    let (index, table) = build_chunk_index_and_key_table(&gens);

    // Shared chunk deduped => 3 unique chunks total (shared, alpha, beta).
    assert_eq!(index.len(), 3);
    assert_eq!(table.entries().len(), 2);

    // index.html: [shared(0), alpha(1)]
    let e0 = &table.entries()[0];
    assert_eq!(e0.chunk_indices, vec![0, 1]);
    assert_eq!(e0.generation, Bytes32([0x11; 32]));

    // about.html: [shared(0), beta(2)] -- reuses shared index 0
    let e1 = &table.entries()[1];
    assert_eq!(e1.chunk_indices, vec![0, 2]);
    assert_eq!(e1.generation, Bytes32([0x22; 32]));
}

#[test]
fn total_size_is_sum_of_chunk_body_lengths() {
    use common::{chunk, FakeGeneration, ResourceSpec};
    let gens = vec![FakeGeneration {
        root: Bytes32([1; 32]),
        generation_id: 1,
        resources: vec![ResourceSpec {
            resource_key: Bytes32([9; 32]),
            chunks: vec![chunk(b"abc"), chunk(b"de")],
        }],
    }];
    let (_index, table) = build_chunk_index_and_key_table(&gens);
    assert_eq!(table.entries()[0].total_size, 5);
}

#[test]
fn lookup_by_resource_key_returns_entry() {
    let gens = common::sample_generations();
    let (_index, table) = build_chunk_index_and_key_table(&gens);
    let rk = common::resource_key("about.html");
    let entry = table.lookup(&rk).expect("about.html present");
    assert_eq!(entry.chunk_indices, vec![0, 2]);
    assert!(table.lookup(&Bytes32([0xFF; 32])).is_none());
}

#[test]
fn verify_against_flags_out_of_range_index() {
    let gens = common::sample_generations();
    let (index, table) = build_chunk_index_and_key_table(&gens);
    // Real count passes.
    assert!(table.verify_against(index.len() as u32).is_ok());
    // Pretend there are fewer chunks than referenced -> MissingChunk(2).
    let err = table.verify_against(2).unwrap_err();
    assert!(matches!(err, digstore_compiler::CompilerError::MissingChunk(2)));
}
