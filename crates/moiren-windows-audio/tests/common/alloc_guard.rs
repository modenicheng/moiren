//! Counting-allocator scaffold for the "allocate and free nothing"
//! real-time-path assertions.
//!
//! Every test binary (and the rt-check harness, which pulls this file in via
//! `#[path]`) compiles its own copy and installs its own
//! [`#[global_allocator`]` static`]: a `#[global_allocator]` exists per binary,
//! so the module cannot be shared as a compiled artifact.
#![allow(dead_code)] // shared across test binaries; each compiles the subset it uses

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct CountingAllocator;

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = COUNTS.try_with(|counts| {
                let (allocated, freed) = counts.get();
                counts.set((allocated + 1, freed));
            });
        }
        // SAFETY: Forward the caller's unchanged allocation contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = COUNTS.try_with(|counts| {
                let (allocated, freed) = counts.get();
                counts.set((allocated, freed + 1));
            });
        }
        // SAFETY: The allocation originated from this System allocator.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `operation` with per-thread allocation counting enabled and returns
/// the observed `(allocated, freed)` counts.
pub fn count_allocations<T>(operation: impl FnOnce() -> T) -> (T, (usize, usize)) {
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    let value = operation();
    TRACK.with(|track| track.set(false));
    (value, COUNTS.with(Cell::get))
}

/// Asserts that `operation` performs no heap allocation and no deallocation.
pub fn assert_allocates_nothing<T>(operation: impl FnOnce() -> T) -> T {
    let (value, counts) = count_allocations(operation);
    assert_eq!(
        counts,
        (0, 0),
        "real-time path allocated or freed: {counts:?}"
    );
    value
}
