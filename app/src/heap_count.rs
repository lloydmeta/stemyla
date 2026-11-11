//! A count of the page's Rust heap (decision 0030, TD-97).
//!
//! On wasm32, the page's global allocator wraps the default one and counts
//! the bytes that the page holds, and their peak.  A save starts the peak
//! again at its start, and logs the peak above the live bytes then at its
//! offer.  `xtask e2e save` compares that peak of a 10-minute and a
//! 1-minute save (D11).  The count does not depend on the place of the
//! free blocks in the memory, so it does not have the two levels of the
//! memory's growth (TD-97).
//!
//! The count allocates nothing and does not wait: each call adds or
//! subtracts with one relaxed atomic operation (RT-1).

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// An allocator that forwards each call to `inner`, and counts the bytes
/// that are live and their peak.
pub struct Counting<A> {
    inner: A,
    live: AtomicUsize,
    peak: AtomicUsize,
}

impl<A> Counting<A> {
    /// A count of zero bytes over `inner`.
    pub const fn new(inner: A) -> Self {
        Self {
            inner,
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// The bytes allocated and not yet freed.
    pub fn live(&self) -> usize {
        self.live.load(Relaxed)
    }

    /// The most bytes that were live since the last [`Self::restart_peak`].
    pub fn peak(&self) -> usize {
        self.peak.load(Relaxed)
    }

    /// Sets the peak to the live bytes, and gives them.
    pub fn restart_peak(&self) -> usize {
        let live = self.live.load(Relaxed);
        self.peak.store(live, Relaxed);
        live
    }

    fn added(&self, bytes: usize) {
        let now = self.live.fetch_add(bytes, Relaxed) + bytes;
        self.peak.fetch_max(now, Relaxed);
    }

    fn removed(&self, bytes: usize) {
        self.live.fetch_sub(bytes, Relaxed);
    }
}

// SAFETY: each call goes unchanged to `inner`, whose contract is the
// caller's.  The count only changes two atomics.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps `alloc`'s contract.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            self.added(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps `alloc_zeroed`'s contract.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.added(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.removed(layout.size());
        // SAFETY: the caller keeps `dealloc`'s contract.
        unsafe { self.inner.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller keeps `realloc`'s contract.
        let moved = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            self.removed(layout.size());
            self.added(new_size);
        }
        moved
    }
}

#[cfg(all(target_arch = "wasm32", not(test)))]
#[global_allocator]
static HEAP: Counting<std::alloc::System> = Counting::new(std::alloc::System);

/// Starts the peak of the page's heap again, and gives the live bytes.
/// `None` where the page has no count: a native build, or a test build.
pub fn restart_peak() -> Option<usize> {
    #[cfg(all(target_arch = "wasm32", not(test)))]
    return Some(HEAP.restart_peak());
    #[cfg(not(all(target_arch = "wasm32", not(test))))]
    None
}

/// The peak of the page's heap since [`restart_peak`], or `None` where
/// the page has no count.
pub fn peak() -> Option<usize> {
    #[cfg(all(target_arch = "wasm32", not(test)))]
    return Some(HEAP.peak());
    #[cfg(not(all(target_arch = "wasm32", not(test))))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::System;

    fn layout(bytes: usize) -> Layout {
        Layout::from_size_align(bytes, 8).expect("a valid layout")
    }

    /// Each call of the allocator moves the live bytes, and the peak keeps
    /// the most since its restart.
    #[test]
    fn the_count_follows_each_call_and_keeps_the_peak() {
        let heap = Counting::new(System);
        // SAFETY: each pointer is freed once, with its own layout.
        unsafe {
            let a = heap.alloc(layout(1_000));
            let b = heap.alloc_zeroed(layout(500));
            assert_eq!((heap.live(), heap.peak()), (1_500, 1_500));
            heap.dealloc(b, layout(500));
            assert_eq!((heap.live(), heap.peak()), (1_000, 1_500));
            let a = heap.realloc(a, layout(1_000), 4_000);
            assert_eq!((heap.live(), heap.peak()), (4_000, 4_000));
            let a = heap.realloc(a, layout(4_000), 200);
            assert_eq!((heap.live(), heap.peak()), (200, 4_000));
            assert_eq!(heap.restart_peak(), 200);
            assert_eq!(heap.peak(), 200);
            let c = heap.alloc(layout(300));
            heap.dealloc(c, layout(300));
            assert_eq!((heap.live(), heap.peak()), (200, 500));
            heap.dealloc(a, layout(200));
            assert_eq!((heap.live(), heap.peak()), (0, 500));
        }
    }

    /// A refused allocation counts nothing.
    #[test]
    fn a_refused_allocation_counts_nothing() {
        struct Refuses;
        // SAFETY: it never gives memory, so nothing is freed.
        unsafe impl GlobalAlloc for Refuses {
            unsafe fn alloc(&self, _: Layout) -> *mut u8 {
                std::ptr::null_mut()
            }
            unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
        }
        let heap = Counting::new(Refuses);
        // SAFETY: `Refuses` gives null, which is not used.
        unsafe {
            assert!(heap.alloc(layout(64)).is_null());
            assert!(heap.alloc_zeroed(layout(64)).is_null());
        }
        assert_eq!((heap.live(), heap.peak()), (0, 0));
    }

    /// A native or a test build has no count of the page's heap.
    #[test]
    fn a_build_with_no_count_gives_none() {
        assert_eq!(restart_peak(), None);
        assert_eq!(peak(), None);
    }
}
