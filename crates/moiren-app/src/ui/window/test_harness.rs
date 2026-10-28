//! Shared harness for app UI and service-facing tests (T4 convergence).
//!
//! Service-level helpers ([`spawn_service`], [`initial_projection`],
//! [`finish_service`], [`wait_until`], [`wait_for`]) cover the in-crate
//! `#[cfg(test)]` suites; the gpui shell builders ([`test_shell`],
//! [`shell_with_state`]) cover the headless window suites gated on the
//! `gpui-test` feature. Integration tests keep a parallel copy of the
//! service tier in `tests/common/mod.rs`: they build against the library
//! with default features, so `#[cfg(test)]` items are not importable there.
//!
//! Every deadline in the migrated suites routes through [`wait_until`] /
//! [`wait_for`]: one poll-then-assert loop, with each call site passing the
//! budget the handwritten loop used before.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::service::{AppHandle, AppSnapshot, ServiceRuntime};
#[cfg(feature = "gpui-test")]
use crate::ui::state::UiState;
use crate::ui::state::initial_snapshot;
#[cfg(feature = "gpui-test")]
use gpui_kit::test::TestWindowExt;

/// Sleep between polls; the deadline, not the interval, is the budget.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Spawns the headless core-driver service used by UI-level tests.
pub fn spawn_service() -> ServiceRuntime {
    ServiceRuntime::spawn().unwrap()
}

/// Blocks for the service's first published snapshot (2s budget, as
/// `ui::state::initial_snapshot`).
pub fn initial_projection(handle: &AppHandle) -> Arc<AppSnapshot> {
    initial_snapshot(handle).unwrap()
}

/// Requests exit and joins the service thread; the only UI-side join.
pub fn finish_service(runtime: ServiceRuntime) {
    runtime.handle().request_exit();
    runtime.join().unwrap();
}

/// Panel springs animate in real time; tests that toggle panels must let the
/// widths settle before clicking handles or measuring canvas geometry.
#[cfg(feature = "gpui-test")]
pub fn settle_panels(cx: &mut gpui_kit::VisualTestContext) {
    std::thread::sleep(Duration::from_millis(900));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    });
}

/// Polls until `poll` returns true, panicking once `timeout` elapses.
#[track_caller]
pub fn wait_until(timeout: Duration, mut poll: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !poll() {
        assert!(
            Instant::now() < deadline,
            "condition unmet within {timeout:?} (called at {:?})",
            std::panic::Location::caller()
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
        assert!(
            Instant::now() < deadline,
            "value unmet within {timeout:?} (called at {:?})",
            std::panic::Location::caller()
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Builds the production shell against a caller-owned service and state
/// (custom drivers, mutated snapshots): init, window, first frame.
#[cfg(feature = "gpui-test")]
pub fn shell_with_state(
    cx: &mut gpui_kit::TestAppContext,
    state: gpui_kit::Entity<UiState>,
) -> (
    gpui_kit::Entity<super::StudioShell>,
    &mut gpui_kit::VisualTestContext,
) {
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| test_shell_body(state, window, cx))
}

/// Standard headless shell: inits gpui, spawns the core-driver service,
/// waits for the initial snapshot and opens the real window view.
#[cfg(feature = "gpui-test")]
pub fn test_shell(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::Entity<super::StudioShell>,
    &mut gpui_kit::VisualTestContext,
    ServiceRuntime,
) {
    use gpui_kit::AppContext;
    cx.update(gpui_kit::init);
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let state = cx.new(|_| UiState::new(handle, snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| test_shell_body(state, window, cx));
    (view, cx, runtime)
}

/// The shell under test must never persist to the developer's real
/// configuration; `StudioShell::new` still reads it for density and language,
/// but preference writes are disabled.
#[cfg(feature = "gpui-test")]
fn test_shell_body(
    state: gpui_kit::Entity<UiState>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<super::StudioShell>,
) -> super::StudioShell {
    let mut shell = super::StudioShell::new(state, window, cx, None);
    shell.preferences_path = None;
    shell
}
