//! WASM ABI helpers: pack/unpack a (ptr, len) pair into an i64 return value.

/// Re-export so `digstore_core::abi::ErrorCode` resolves (consumed by host/guest).
pub use crate::error::ErrorCode;

/// Pack a 32-bit pointer and length into a single i64: `(ptr << 32) | len`.
pub const fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

/// Split a packed i64 back into `(ptr, len)`.
pub const fn unpack_ptr_len(packed: i64) -> (u32, u32) {
    let ptr = (packed >> 32) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}

/// An error sentinel has `len == 0` and the pointer reinterpreted as i32 is negative.
pub const fn is_error(packed: i64) -> bool {
    let (ptr, len) = unpack_ptr_len(packed);
    len == 0 && (ptr as i32) < 0
}
