//! A minimal bump allocator used as the wasm `#[global_allocator]`.
//! On native test builds we keep using the system allocator; the bump
//! allocator is still exercised directly by unit tests for distinctness.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

const HEAP_SIZE: usize = 8 * 1024 * 1024;

pub struct BumpAllocator {
    heap: UnsafeCell<[u8; HEAP_SIZE]>,
    next: AtomicUsize,
}

unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator {
            heap: UnsafeCell::new([0u8; HEAP_SIZE]),
            next: AtomicUsize::new(0),
        }
    }

    /// Bump-allocate `layout`, returning a pointer into the static heap, or null on OOM.
    pub fn bump(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let base = self.heap.get() as *mut u8 as usize;
        loop {
            let cur = self.next.load(Ordering::Relaxed);
            let start = base + cur;
            let aligned = (start + align - 1) & !(align - 1);
            let new_cur = aligned - base + size;
            if new_cur > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            if self
                .next
                .compare_exchange(cur, new_cur, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
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
    use alloc::boxed::Box;
    use core::alloc::Layout;

    // DEVIATION (test-only, Windows): the 8 MiB heap is built DIRECTLY on the
    // heap (no stack temporary) so constructing `BumpAllocator` does not overflow
    // the small test thread stack. Behaviour is identical to the wasm static.
    fn boxed_allocator() -> Box<BumpAllocator> {
        // Allocate zeroed heap storage and reinterpret as BumpAllocator. The heap
        // is `[u8; HEAP_SIZE]` (all-zero is a valid `next == 0`), so a zeroed box
        // is a fully valid `BumpAllocator`.
        unsafe {
            let layout = core::alloc::Layout::new::<BumpAllocator>();
            let ptr = alloc::alloc::alloc_zeroed(layout) as *mut BumpAllocator;
            assert!(!ptr.is_null());
            Box::from_raw(ptr)
        }
    }

    #[test]
    fn bump_returns_distinct_aligned_pointers() {
        let a = boxed_allocator();
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
        let a = boxed_allocator();
        let huge = Layout::from_size_align(super::HEAP_SIZE + 1, 1).unwrap();
        assert!(a.bump(huge).is_null());
    }
}
