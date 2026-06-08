//! Default data-region symbols so the template links before the compiler injects
//! the real data section. The compiler overwrites this segment with the real
//! `DIGS`-framed bytes. Wasm-only.
//!
//! `__digstore_data` holds a valid, empty section (magic `DIGS`, version 1, zero
//! sections); `__digstore_data_end` marks the end so `datasection::embedded()`
//! can compute the length as `&end - &start`.

/// A valid empty data section: magic(4) + version(1) + section_count(u32 BE = 0).
#[no_mangle]
pub static __digstore_data: [u8; 9] = *b"DIGS\x01\x00\x00\x00\x00";

/// End marker immediately following the data region.
#[no_mangle]
pub static __digstore_data_end: u8 = 0;
