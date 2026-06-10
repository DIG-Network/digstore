use digstore_core::codec::{Decode, Encode};
use digstore_core::tombstone::{RevocationReason, Tombstone, TombstoneScope};
use digstore_core::Bytes32;

fn assert_roundtrip<T: Encode + Decode + PartialEq + core::fmt::Debug>(value: T) {
    let bytes = value.to_bytes();
    let decoded = T::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded, value);
}

#[test]
fn root_scoped_tombstone_round_trips() {
    assert_roundtrip(Tombstone::root(
        Bytes32([1; 32]),
        Bytes32([2; 32]),
        1_700_000_000,
        RevocationReason::Compromise,
    ));
}

#[test]
fn store_scoped_tombstone_round_trips() {
    assert_roundtrip(Tombstone::store(
        Bytes32([9; 32]),
        42,
        RevocationReason::Takedown,
    ));
}

#[test]
fn canonical_layout_is_stable_for_root_scope() {
    let t = Tombstone {
        store_id: Bytes32([0xAA; 32]),
        scope: TombstoneScope::Root(Bytes32([0xBB; 32])),
        not_after: 0x0102_0304_0506_0708,
        reason: RevocationReason::Superseded.as_u8(),
    };
    let b = t.canonical();
    // store_id(32) || scope_tag(1=0 for Root) || root(32) || not_after_be(8) || reason(1)
    assert_eq!(b.len(), 32 + 1 + 32 + 8 + 1);
    assert_eq!(&b[0..32], &[0xAA; 32]);
    assert_eq!(b[32], 0, "Root scope tag is 0");
    assert_eq!(&b[33..65], &[0xBB; 32]);
    assert_eq!(
        &b[65..73],
        &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        "not_after is big-endian"
    );
    assert_eq!(b[73], 2, "Superseded reason byte");
}

#[test]
fn canonical_layout_is_stable_for_store_scope() {
    let t = Tombstone::store(Bytes32([0xCC; 32]), 7, RevocationReason::Unspecified);
    let b = t.canonical();
    // store_id(32) || scope_tag(1=1 for Store) || not_after_be(8) || reason(1)
    // (NO root field for the Store scope).
    assert_eq!(b.len(), 32 + 1 + 8 + 1);
    assert_eq!(b[32], 1, "Store scope tag is 1");
}

#[test]
fn reason_byte_round_trips_known_values() {
    for (r, byte) in [
        (RevocationReason::Unspecified, 0u8),
        (RevocationReason::Compromise, 1),
        (RevocationReason::Superseded, 2),
        (RevocationReason::Takedown, 3),
    ] {
        assert_eq!(r.as_u8(), byte);
        assert_eq!(RevocationReason::from_u8(byte), r);
    }
    // Unknown bytes degrade to Unspecified for the human label.
    assert_eq!(RevocationReason::from_u8(200), RevocationReason::Unspecified);
}

#[test]
fn changing_any_field_changes_canonical_bytes() {
    let base = Tombstone::root(
        Bytes32([1; 32]),
        Bytes32([2; 32]),
        100,
        RevocationReason::Compromise,
    );
    let diff_store = Tombstone::root(
        Bytes32([3; 32]),
        Bytes32([2; 32]),
        100,
        RevocationReason::Compromise,
    );
    let diff_root = Tombstone::root(
        Bytes32([1; 32]),
        Bytes32([4; 32]),
        100,
        RevocationReason::Compromise,
    );
    let diff_time = Tombstone::root(
        Bytes32([1; 32]),
        Bytes32([2; 32]),
        101,
        RevocationReason::Compromise,
    );
    let diff_reason = Tombstone::root(
        Bytes32([1; 32]),
        Bytes32([2; 32]),
        100,
        RevocationReason::Takedown,
    );
    let c = base.canonical();
    assert_ne!(c, diff_store.canonical());
    assert_ne!(c, diff_root.canonical());
    assert_ne!(c, diff_time.canonical());
    assert_ne!(c, diff_reason.canonical());
}
