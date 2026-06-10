//! A minimal bump allocator used as the wasm `#[global_allocator]`.
//! On native test builds we keep using the system allocator; the bump
//! allocator is still exercised directly by unit tests for distinctness.
//!
//! ## BINDING contract D2 — heap must not collide with the injected data section
//!
//! The compiler injects the `DIGS` data-section blob as an **active data
//! segment** at the fixed linear-memory offset
//! [`digstore_core::datasection::DIGS_DATA_OFFSET`] (2 MiB), and the guest reads
//! the pool/key-table directly from that pointer. If the heap that backs
//! `get_content`'s response building (chunk copies, `concat_output`, response
//! serialization) overlapped that region, growing allocations would **overwrite
//! the injected chunk pool**, and the module would serve only the chunks read
//! before the corruption — i.e. it would NOT serve itself for multi-chunk /
//! large resources (observed: only the first ~2 chunks survived).
//!
//! Therefore the bump heap starts at a **dynamic** base computed *above* the
//! injected blob: on wasm32 the first allocation reads the `DIGS` header at
//! [`digstore_core::datasection::DIGS_DATA_OFFSET`] and places the heap at
//! `align_up(DIGS_DATA_OFFSET + total_blob_len, 64 KiB)`, so a store of ANY size
//! (up to the §5.1 128 MiB ceiling) gets a heap clear of its own data section.
//! When no `DIGS` header is present (native test builds, or an absent blob) the
//! heap falls back to a fixed [`FALLBACK_HEAP_BASE`] (8 MiB), still above the
//! data-section window. There is no hard upper cap inside the allocator: OOM is
//! signaled only when `memory.grow` fails (the host enforces the outer ceiling).

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Fixed fallback heap base used on native (test) builds and when no `DIGS`
/// header is present in linear memory. Above the data-section window so heap
/// growth can never corrupt an (absent) blob (contract D2). 8 MiB.
pub const FALLBACK_HEAP_BASE: usize = 8 * 1024 * 1024;
/// Wasm page size. Referenced by the wasm32 `ensure_memory` path and the pure
/// `heap_base_from_total_len` helper (the latter only on wasm32 / in tests).
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
const WASM_PAGE: usize = 65536;

/// `heap_base = align_up(DIGS_DATA_OFFSET + total_blob_len, 64 KiB)`.
/// Used by the wasm32 init path (`resolve_heap_base`) and the unit tests.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#[inline]
fn heap_base_from_total_len(total_len: usize) -> usize {
    let off = digstore_core::datasection::DIGS_DATA_OFFSET as usize;
    let end = off + total_len;
    let page = WASM_PAGE;
    end.div_ceil(page) * page
}

pub struct BumpAllocator {
    /// Next free byte as an ABSOLUTE linear-memory address. `0` is the
    /// "uninitialized" sentinel: the first `bump` installs the dynamic base.
    next: AtomicUsize,
}

unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator {
            next: AtomicUsize::new(0), // 0 = uninitialized
        }
    }

    /// Compute the dynamic heap base by reading the injected `DIGS` header at
    /// `DIGS_DATA_OFFSET`. Returns the fallback if magic/version are absent.
    #[cfg(target_arch = "wasm32")]
    fn resolve_heap_base() -> usize {
        use digstore_core::datasection::DIGS_DATA_OFFSET;
        const HEADER_LEN: usize = 9; // magic(4)+version(1)+count(u32 BE)
        const ROW_LEN: usize = 10; // id(u16)+offset(u32)+len(u32)
        unsafe {
            let base = DIGS_DATA_OFFSET as *const u8;
            let header = core::slice::from_raw_parts(base, HEADER_LEN);
            if &header[0..4] != b"DIGS" || header[4] != 1 {
                return FALLBACK_HEAP_BASE;
            }
            let count = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
            let table_len = match count
                .checked_mul(ROW_LEN)
                .and_then(|t| t.checked_add(HEADER_LEN))
            {
                Some(n) => n,
                None => return FALLBACK_HEAP_BASE,
            };
            let rows = core::slice::from_raw_parts(base, table_len);
            let mut total_len = table_len;
            for i in 0..count {
                let p = HEADER_LEN + i * ROW_LEN;
                let offset =
                    u32::from_be_bytes([rows[p + 2], rows[p + 3], rows[p + 4], rows[p + 5]])
                        as usize;
                let len = u32::from_be_bytes([rows[p + 6], rows[p + 7], rows[p + 8], rows[p + 9]])
                    as usize;
                match offset.checked_add(len) {
                    Some(end) if end > total_len => total_len = end,
                    Some(_) => {}
                    None => return FALLBACK_HEAP_BASE,
                }
            }
            heap_base_from_total_len(total_len)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_heap_base() -> usize {
        // Native test builds have no real linear memory and no injected blob; the
        // init path must never dereference memory, so use the fixed fallback.
        FALLBACK_HEAP_BASE
    }

    /// Ensure linear memory covers byte address `end` (wasm only); grow if short.
    #[cfg(target_arch = "wasm32")]
    fn ensure_memory(end: usize) -> bool {
        let have_pages = core::arch::wasm32::memory_size(0);
        let have_bytes = have_pages * WASM_PAGE;
        if end <= have_bytes {
            return true;
        }
        let need_pages = (end - have_bytes).div_ceil(WASM_PAGE);
        // memory.grow returns the previous size, or usize::MAX on failure.
        core::arch::wasm32::memory_grow(0, need_pages) != usize::MAX
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_memory(_end: usize) -> bool {
        // Native test builds use the system allocator; bump() is unit-tested with
        // explicit addresses only and never dereferences linear memory.
        true
    }

    /// Bump-allocate `layout`, returning an absolute linear-memory address, or
    /// null on overflow or failed memory growth (OOM). The heap base is installed
    /// lazily on the first allocation (dynamic, above the data section).
    pub fn bump(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(1);
        let size = layout.size();
        loop {
            let mut cur = self.next.load(Ordering::Relaxed);
            if cur == 0 {
                // First allocation: install the dynamic base. Losers re-read.
                let base = Self::resolve_heap_base();
                match self
                    .next
                    .compare_exchange(0, base, Ordering::SeqCst, Ordering::Relaxed)
                {
                    Ok(_) => cur = base,
                    Err(observed) => cur = observed,
                }
            }
            let aligned = (cur + align - 1) & !(align - 1);
            let end = match aligned.checked_add(size) {
                Some(e) => e,
                None => return core::ptr::null_mut(),
            };
            if self
                .next
                .compare_exchange(cur, end, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                if !Self::ensure_memory(end) {
                    return core::ptr::null_mut();
                }
                return aligned as *mut u8;
            }
        }
    }
}

impl Default for BumpAllocator {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.bump(layout)
    }
    // Bump allocator never frees individual allocations.
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[cfg(all(target_arch = "wasm32", not(feature = "std")))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator::new();

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    #[test]
    fn fallback_base_is_8mib_without_a_digs_header() {
        // Native build: no DIGS magic in memory, so the fixed fallback applies.
        let a = BumpAllocator::new();
        let p1 = a.bump(Layout::from_size_align(64, 8).unwrap());
        assert!(!p1.is_null());
        assert!((p1 as usize) >= FALLBACK_HEAP_BASE);
        assert!(FALLBACK_HEAP_BASE > digstore_core::datasection::DIGS_DATA_OFFSET as usize);
    }

    #[test]
    fn bump_returns_distinct_aligned_pointers() {
        let a = BumpAllocator::new();
        let p1 = a.bump(Layout::from_size_align(64, 8).unwrap()) as usize;
        let p2 = a.bump(Layout::from_size_align(64, 8).unwrap()) as usize;
        assert_ne!(p1, p2);
        assert!(p2 >= p1 + 64);
        assert_eq!(p1 % 8, 0);
        assert_eq!(p2 % 8, 0);
    }

    #[test]
    fn heap_base_from_header_sits_above_the_blob() {
        // Unit-test the pure computation: given a total blob length, the base is
        // page-aligned and strictly above DIGS_DATA_OFFSET + total_len.
        let off = digstore_core::datasection::DIGS_DATA_OFFSET as usize;
        let total_len = 10 * 1024 * 1024; // 10 MiB blob (over the old 8 MiB base)
        let base = heap_base_from_total_len(total_len);
        assert!(base >= off + total_len);
        assert_eq!(base % 65536, 0);
    }
}
