use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    alloc: usize,
    realloc: usize,
    free: usize,
}
thread_local! { static PROBE: Cell<Option<Counts>> = const { Cell::new(None) }; }
pub(crate) struct Allocator;
fn record(kind: usize) {
    let _ = PROBE.try_with(|state| {
        if let Some(mut counts) = state.get() {
            match kind {
                0 => counts.alloc += 1,
                1 => counts.realloc += 1,
                _ => counts.free += 1,
            }
            state.set(Some(counts));
        }
    });
}
// SAFETY: All operations delegate the original pointer/layout unchanged
// to System. Preinitialized TLS counters do not allocate recursively.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(0);
        // SAFETY: Forward the caller's valid allocation layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(1);
        // SAFETY: Forward the original allocation and requested size.
        unsafe { System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(2);
        // SAFETY: Forward the original allocation and layout.
        unsafe { System.dealloc(pointer, layout) };
    }
}
pub(crate) fn measure(operation: impl FnOnce()) -> Counts {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            PROBE.with(|state| state.set(None));
        }
    }
    PROBE.with(|state| state.set(Some(Counts::default())));
    let stop = Stop;
    operation();
    let counts = PROBE.with(|state| state.get().unwrap());
    drop(stop);
    counts
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
