use digstore_compiler::ChunkIndex;
use digstore_core::Bytes32;

fn h(b: u8) -> Bytes32 {
    Bytes32([b; 32])
}

#[test]
fn inserts_assign_sequential_indices() {
    let mut idx = ChunkIndex::new();
    assert_eq!(idx.insert(h(1), vec![0xAA]), 0);
    assert_eq!(idx.insert(h(2), vec![0xBB]), 1);
    assert_eq!(idx.len(), 2);
}

#[test]
fn duplicate_hash_returns_existing_index_and_does_not_grow() {
    let mut idx = ChunkIndex::new();
    let first = idx.insert(h(7), vec![0x01, 0x02]);
    let again = idx.insert(h(7), vec![0x01, 0x02]);
    assert_eq!(first, again);
    assert_eq!(idx.len(), 1);
}

#[test]
fn bodies_returned_in_insertion_order() {
    let mut idx = ChunkIndex::new();
    idx.insert(h(3), vec![0x30]);
    idx.insert(h(1), vec![0x10]);
    idx.insert(h(2), vec![0x20]);
    let bodies: Vec<Vec<u8>> = idx.bodies_in_order().map(|b| b.to_vec()).collect();
    assert_eq!(bodies, vec![vec![0x30], vec![0x10], vec![0x20]]);
}

#[test]
fn index_of_resolves_known_hash() {
    let mut idx = ChunkIndex::new();
    idx.insert(h(5), vec![0x55]);
    idx.insert(h(6), vec![0x66]);
    assert_eq!(idx.index_of(&h(6)), Some(1));
    assert_eq!(idx.index_of(&h(9)), None);
}
