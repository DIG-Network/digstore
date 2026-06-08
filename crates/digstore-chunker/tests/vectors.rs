use digstore_chunker::GEAR_TABLE;

#[test]
fn gear_table_has_256_entries() {
    assert_eq!(GEAR_TABLE.len(), 256);
}

#[test]
fn gear_table_is_nontrivial() {
    // Not the all-zero placeholder from the scaffold.
    assert!(GEAR_TABLE.iter().any(|&x| x != 0), "gear table must not be all zero");
    // High-quality table: every entry distinct so no two bytes alias.
    let mut seen = std::collections::HashSet::new();
    for &v in GEAR_TABLE.iter() {
        assert!(seen.insert(v), "gear table entries must be distinct, found dup {v:#018x}");
    }
}

#[test]
fn gear_table_pinned_guards_are_present() {
    // Pin two values so the table can never silently change (determinism guard).
    assert_eq!(GEAR_TABLE[0], 0x3b5c_9f8e_2d71_a046);
    assert_eq!(GEAR_TABLE[255], 0x9e1d_4a7c_60b3_82f5);
}
