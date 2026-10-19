//! Shared integration-test scaffolding for moiren-app (T4 convergence).
//!
//! Every `tests/*.rs` binary compiles its own copy via `mod common;`. These
//! targets build against the library with default features, so the in-crate
//! `#[cfg(test)]` harness in `ui::window::test_harness` is not importable
//! here; the deadline semantics (poll-then-assert, 1 ms interval, explicit
//! per-site budget) mirror it exactly.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use moiren_app::service::{AppHandle, AppSnapshot, ServiceRuntime};

/// Sleep between polls; the deadline, not the interval, is the budget.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Spawns the headless core-driver service.
pub fn service() -> ServiceRuntime {
    ServiceRuntime::spawn().unwrap()
}

/// Blocks for the service's first published snapshot (2s budget, matching
/// the UI-side `initial_snapshot`).
pub fn wait_snapshot(handle: &AppHandle) -> Arc<AppSnapshot> {
    wait_for(Duration::from_secs(2), || handle.try_snapshot())
}

/// Requests exit and joins the service thread; the only UI-side join.
pub fn finish(runtime: ServiceRuntime) {
    runtime.handle().request_exit();
    runtime.join().unwrap();
}

/// Polls until `poll` returns true, panicking once `timeout` elapses.
#[track_caller]
pub fn wait_until(timeout: Duration, mut poll: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !poll() {
        assert!(
            Instant::now() < deadline,
            "condition unmet within {timeout:?}"
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Polls until `poll` yields a value, panicking once `timeout` elapses.
#[track_caller]
pub fn wait_for<T>(timeout: Duration, mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "value unmet within {timeout:?}");
        std::thread::sleep(POLL_INTERVAL);
    }
}
