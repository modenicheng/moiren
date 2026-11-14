//! Windows audio probes, Shared capture/render and backend clock adaptation.
//!
//! The canonical thread/realtime-boundary map for this crate — every thread
//! that drives audio, what each may do, and which module owns it — lives in
//! `docs/platform/windows-audio-threads.md`. Two test files are deliberately
//! absent from every crate target and compile only under `tools/rt-check`
//! (`#[path]` shadow mounts): `asio_rt_tests.rs` and
//! `engine_backend/recovery_host_tests.rs`.

#[cfg(all(windows, target_pointer_width = "64"))]
pub mod asio;
#[cfg(windows)]
pub(crate) mod bridge;
pub mod capture;
#[cfg(any(windows, test))]
pub(crate) mod capture_packet;
// Offline clock-fit analysis consumed only by the probe-tools `physical`
// module; keeping it gated avoids exporting a probe-only analysis surface
// from library builds.
#[cfg(all(windows, feature = "probe-tools"))]
pub mod clock;
pub mod clock_bridge;
pub(crate) mod codec;
mod duration;
#[cfg(windows)]
pub mod engine_backend;
pub(crate) mod format;
pub mod latency;
pub mod native;
pub mod process_loopback;
pub mod render_evidence;
pub(crate) mod resample;
pub(crate) mod servo;
pub mod stats;
pub mod vad;
#[cfg(windows)]
pub mod wasapi;

pub mod virtual_owner;
pub use duration::{DurationError, SessionDuration};

/// Native WASAPI/virtual-endpoint sample rate every owner in this crate
/// targets. The engine may still run at another rate; owners adapt through
/// the clock bridge and boundary resamplers.
pub(crate) const NATIVE_SAMPLE_RATE: u32 = 48_000;

/// Per-resource teardown deadline for native owners (issue #105): healthy
/// stop/release/drop runs in tens of milliseconds per device (ASIO resets
/// the slowest healthy case on record, well under a second), so 2 s bounds
/// one wedged driver wind-down at a still-responsive Stop without falsely
/// abandoning a slow-but-alive teardown — and stays tighter than the 10 s
/// open bound because teardown blocks a user-initiated Stop or app exit.
///
/// Windows-only like both of its users (the professional and virtual
/// owners); other platforms compile no native owner teardown.
#[cfg(windows)]
pub(crate) const NATIVE_TEARDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(windows)]
mod session;
#[cfg(windows)]
pub use session::{Activation, ActivationGate, GateError, StopSignal};

// Example-only probe tooling. These modules drive real WASAPI/SWDevice COM
// surfaces exclusively for the `w00_*`/`exclusive_probe` examples; they are
// kept out of library builds (and out of consumer dependency graphs) behind
// the opt-in `probe-tools` feature. Enable it to build the probe examples.
#[cfg(all(windows, feature = "probe-tools"))]
pub mod probe;

#[cfg(all(windows, feature = "probe-tools"))]
pub mod exclusive_probe;

#[cfg(all(windows, feature = "probe-tools"))]
pub mod physical;

#[cfg(all(windows, feature = "probe-tools"))]
pub mod swdevice;

#[cfg(windows)]
mod catalog;
#[cfg(windows)]
pub use catalog::{
    CatalogSnapshot, DefaultEndpoint, EndpointSnapshot, owned_snapshot as catalog_snapshot,
};
#[cfg(windows)]
mod win32_raii;

#[cfg(windows)]
pub(crate) mod negotiation;

// One counting allocator per library-test binary, shared by bounded hot-path
// regressions. Integration-test binaries install their own copy.
#[cfg(test)]
#[path = "../tests/common/alloc_guard.rs"]
pub(crate) mod test_allocation;
