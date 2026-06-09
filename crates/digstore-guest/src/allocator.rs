//! A minimal bump allocator used as the wasm `#[global_allocator]`.
//! On native test builds we keep using the system allocator; the bump
//! allocator is still exercised directly by unit tests for distinctness.
//!
//! ## BINDING contract D2 — heap must not collide with the injected data section
//!
//! The compiler injects the `DIGS` data-section blob as an **active data
//! segment** at the fixed linear-memory offset
//! [`digstore_core::datasection::DIGS_DATA_OFFSET`] (1 MiB), and the guest reads
//! the pool/key-table directly from that pointer. If the heap that backs
//! `get_content`'s response building (chunk copies, `concat_output`, response
//! serialization) overlapped that region, growing allocations would **overwrite
//! the injected chunk pool**, and the module would serve only the chunks read
//! before the corruption — i.e. it would NOT serve itself for multi-chunk /
//! large resources (observed: only the first ~2 chunks survived).
//!
//! Therefore the bump heap starts at [`HEAP_BASE`], chosen **above** the blob
//! window `[DIGS_DATA_OFFSET, HEAP_BASE)`, and is backed by linear-memory
//! `memory.grow` on demand (not by a static array that the linker would place
//! right at the data-section offset). The window `[DIGS_DATA_OFFSET, HEAP_BASE)`
//! (1 MiB .. 8 MiB) reserves room for the injected blob; the heap occupies
//! `[HEAP_BASE, 16 MiB)` (the §5.1 module memory ceiling).

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

/// First heap address. Above the injected `DIGS` blob window
/// `[DIGS_DATA_OFFSET, HEAP_BASE)` so heap growth never corrupts the data section
/// (see module docs / contract D2). 8 MiB.
pub const HEAP_BASE: usize = 8 * 1024 * 1024;
/// Upper bound of the heap = the §5.1 module memory ceiling (16 MiB). Allocations
/// beyond this fail (null), matching the host's `memory_bytes_max` limiter.
pub const HEAP_END: usize = 16 * 1024 * 1024;
/// Wasm page size.
const WASM_PAGE: usize = 65536;

pub struct BumpAllocator {
    /// Next free byte as an ABSOLUTE linear-memory address (starts at `HEAP_BASE`).
    next: AtomicUsize,
}

unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator {
            next: AtomicUsize::new(HEAP_BASE),
        }
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
    /// null on OOM (past `HEAP_END`) or failed memory growth.
    pub fn bump(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(1);
        let size = layout.size();
        loop {
            let cur = self.next.load(Ordering::Relaxed);
            let aligned = (cur + align - 1) & !(align - 1);
            let end = match aligned.checked_add(size) {
                Some(e) => e,
                None => return core::ptr::null_mut(),
            };
            if end > HEAP_END {
                return core::ptr::null_mut();
            }
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
    fn bump_starts_above_the_data_section_window() {
        // The first allocation must land at/above HEAP_BASE, i.e. ABOVE the
        // injected DIGS blob window so heap growth can never corrupt the data
        // section (contract D2). HEAP_BASE must be above DIGS_DATA_OFFSET.
        assert!(HEAP_BASE > digstore_core::datasection::DIGS_DATA_OFFSET as usize);
        let a = BumpAllocator::new();
        let p1 = a.bump(Layout::from_size_align(64, 8).unwrap());
        assert!(!p1.is_null());
        assert!(
            (p1 as usize) >= HEAP_BASE,
            "first allocation must be at/above HEAP_BASE (above the blob window)"
        );
    }

    #[test]
    fn bump_returns_distinct_aligned_pointers() {
        let a = BumpAllocator::new();
        let l = Layout::from_size_align(64, 8).unwrap();
        let p1 = a.bump(l);
        let p2 = a.bump(l);
        assert!(!p1.is_null() && !p2.is_null());
        assert_ne!(p1, p2, "two allocations must not alias");
        assert_eq!((p1 as usize) % 8, 0, "p1 must be 8-aligned");
        assert_eq!((p2 as usize) % 8, 0, "p2 must be 8-aligned");
        assert!((p2 as usize) >= (p1 as usize) + 64, "p2 must be past p1's region");
    }

    #[test]
    fn bump_oom_returns_null() {
        let a = BumpAllocator::new();
        // A request larger than the whole heap window must fail.
        let huge = Layout::from_size_align(HEAP_END - HEAP_BASE + 1, 1).unwrap();
        assert!(a.bump(huge).is_null());
    }
}
