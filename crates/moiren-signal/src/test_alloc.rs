// Single owner of the counting-allocator semantics. This file is compiled
// into the library's unit-test binary (`#[cfg(test)] mod test_alloc`) and is
// textually included by `tests/allocation.rs`, which compiles the library
// without `cfg(test)` and therefore cannot import it. Keep it self-contained.
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
// Frees tracking is consumed only by unit tests inside the library; it stays
// dead in the integration binary that includes this file.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) struct Frees {
    pub(crate) events: [Option<(usize, std::thread::ThreadId)>; 256],
    pub(crate) count: usize,
}
thread_local! { static FREES: Cell<Option<Frees>> = const { Cell::new(None) }; }
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
        let _ = FREES.try_with(|probe| {
            if let Some(mut frees) = probe.get() {
                assert!(frees.count < frees.events.len());
                frees.events[frees.count] = Some((pointer as usize, std::thread::current().id()));
                frees.count += 1;
                probe.set(Some(frees));
            }
        });
    }
}

#[allow(dead_code)]
pub(crate) fn frees(operation: impl FnOnce()) -> Frees {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            FREES.with(|probe| probe.set(None));
        }
    }
    FREES.with(|probe| {
        probe.set(Some(Frees {
            events: [None; 256],
            count: 0,
        }))
    });
    let stop = Stop;
    operation();
    let frees = FREES.with(|probe| probe.get().unwrap());
    drop(stop);
    frees
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
