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
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(super) fn track_allocations<R>(f: impl FnOnce() -> R) -> (R, (usize, usize)) {
    struct Tracking;
    impl Drop for Tracking {
        fn drop(&mut self) {
            // A panic must not leave counting enabled on a reused test thread.
            TRACK.with(|track| track.set(false));
        }
    }

    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    let tracking = Tracking;
    let result = f();
    drop(tracking);
    (result, COUNTS.with(Cell::get))
}
