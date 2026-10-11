use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

// Counts only this test thread while render is active; unrelated parallel test
// allocations cannot contaminate the result. TLS has constant initialization.
struct CountingAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static REALLOCS: Cell<usize> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a + 1, d));
                });
            }
        });
        // SAFETY: forwarding the caller's allocation contract unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a, d + 1));
                });
            }
        });
        // SAFETY: ptr/layout originated from this allocator's System call.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = REALLOCS.try_with(|counts| counts.set(counts.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(super) fn track_allocations<R>(f: impl FnOnce() -> R) -> (R, (usize, usize)) {
    let (result, (allocations, reallocations, deallocations)) = track_heap(f);
    (
        result,
        (allocations + reallocations, deallocations + reallocations),
    )
}

pub(super) fn track_heap<R>(f: impl FnOnce() -> R) -> (R, (usize, usize, usize)) {
    struct Tracking;
    impl Drop for Tracking {
        fn drop(&mut self) {
            // A panic must not leave counting enabled on a reused test thread.
            TRACK.with(|track| track.set(false));
        }
    }

    COUNTS.with(|counts| counts.set((0, 0)));
    REALLOCS.with(|counts| counts.set(0));
    TRACK.with(|track| track.set(true));
    let tracking = Tracking;
    let result = f();
    drop(tracking);
    let (allocations, deallocations) = COUNTS.with(Cell::get);
    (
        result,
        (allocations, REALLOCS.with(Cell::get), deallocations),
    )
}

#[test]
fn c3_allocator_distinguishes_reallocation_and_keeps_pair_detection() {
    let mut buffer = Vec::<u64>::with_capacity(1);
    buffer.push(1);
    let (_, counts) = track_heap(|| buffer.reserve_exact(16));
    assert_eq!(counts, (0, 1, 0));
    let (_, counts) = track_allocations(|| buffer.reserve_exact(64));
    assert_eq!(counts, (1, 1));
}
