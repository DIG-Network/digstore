use digstore_core::{is_error, pack_ptr_len, unpack_ptr_len};

#[test]
fn pack_unpack_roundtrip() {
    let cases = [
        (0u32, 0u32),
        (1, 64),
        (0x0001_0000, 0xFFFF),
        (0xDEAD_BEEF, 1024),
    ];
    for (ptr, len) in cases {
        let packed = pack_ptr_len(ptr, len);
        assert_eq!(unpack_ptr_len(packed), (ptr, len));
    }
}

#[test]
fn pack_layout_is_high_ptr_low_len() {
    // ptr=1, len=2 => (1<<32)|2
    assert_eq!(pack_ptr_len(1, 2), (1i64 << 32) | 2);
}

#[test]
fn is_error_sentinel() {
    // Error sentinel: len == 0 && (ptr as i32) < 0.
    // ptr=0x8000_0000 has high bit set => negative i32, len 0 => error.
    let err = pack_ptr_len(0x8000_0000, 0);
    assert!(is_error(err));
    // A normal zero-length success at ptr=0 is NOT an error.
    assert!(!is_error(pack_ptr_len(0, 0)));
    // Non-zero length is never an error regardless of ptr.
    assert!(!is_error(pack_ptr_len(0x8000_0000, 5)));
    // An ErrorCode packed as ptr with len 0 is an error (codes are negative i32).
    let code_packed = pack_ptr_len((-1i32) as u32, 0);
    assert!(is_error(code_packed));
}
