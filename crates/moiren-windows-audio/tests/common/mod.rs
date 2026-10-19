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
