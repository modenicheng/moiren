//! Shared integration-test scaffolding for moiren-windows-audio.
//!
//! Every `tests/*.rs` binary compiles its own copy via `mod common;` — the
//! rt-check harness includes `alloc_guard.rs` directly via `#[path]` because
//! it lives in a separate crate.
pub mod alloc_guard;
pub mod clock;
pub mod clock_sim;

// Each test binary uses a subset of the re-exported helpers.
#[allow(unused_imports)]
pub use alloc_guard::assert_allocates_nothing;
#[allow(unused_imports)]
pub use clock_sim::{OUTPUT_RATE, burst_delay, capture_interval, capture_stamp, fill_ramp};

/// Runs independent simulation cases across the available cores.
///
/// The clock suites are Cartesian products of frame-domain simulations that
/// share no state, so a serial loop leaves most of the runner idle for many
/// minutes. Cases must stay independent and `body` must only touch `Sync`
/// state; a panicking worker fails the test through the scoped-thread join.
/// `println!` stays line-atomic under concurrency, so the per-case CSV lines
/// remain intact even though their order is now scheduler-dependent.
// Each test binary compiles its own copy of this module and uses only part of
// it, matching the allow on the re-exports above.
#[allow(dead_code)]
pub fn for_each_parallel<T, F>(cases: &[T], body: F)
where
    T: Sync,
    F: Fn(&T) + Send + Sync,
{
    use std::sync::atomic::{AtomicUsize, Ordering};

    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(cases.len())
        .max(1);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(index) else { break };
                    body(case);
                }
            });
        }
    });
}
