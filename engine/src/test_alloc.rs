//! The heap counter of the engine's unit tests.  A test measures the peak
//! of the heap above its start with [`peak_of`].  Each thread keeps its own
//! count, so tests that run at the same time do not change each other's.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    /// Heap bytes this thread allocated and has not freed.
    static LIVE: Cell<usize> = const { Cell::new(0) };
    /// The most `LIVE` has been since a test set it.
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

/// Keeps `LIVE` and `PEAK` for each thread.  Only `alloc` and `dealloc`
/// are written, so the default `realloc` holds the old and the new
/// buffer at once, as `System`'s may.
struct PeakAllocator;

// SAFETY: every call is forwarded unchanged to `System`; counting
// touches only `const` thread-locals, which do not allocate.
unsafe impl GlobalAlloc for PeakAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = LIVE.try_with(|live| {
            let now = live.get() + layout.size();
            live.set(now);
            let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
        });
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(layout.size())));
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

/// Runs `run`, and gives the most heap bytes that this thread held above
/// the start while it ran, with its result.
pub fn peak_of<T>(run: impl FnOnce() -> T) -> (usize, T) {
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let out = run();
    (PEAK.with(Cell::get) - before, out)
}
