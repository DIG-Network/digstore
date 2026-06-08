//! Guest-side ptr/len packing. MUST be byte-identical to `digstore_core::abi`.
//! Re-derived here (not just re-exported) so the wasm ABI layer has no_std-clean
//! const fns, and parity is enforced by `tests/abi_roundtrip.rs`.

/// Pack (ptr, len) into the i64 ABI return value.
pub const fn guest_pack(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

/// Inverse of `guest_pack`.
pub const fn guest_unpack(packed: i64) -> (u32, u32) {
    let ptr = (packed >> 32) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}
