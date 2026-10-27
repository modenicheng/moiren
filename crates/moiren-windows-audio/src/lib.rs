//! Windows audio probes, Shared capture/render and backend clock adaptation.

pub mod capture;
#[cfg(any(windows, test))]
pub(crate) mod capture_packet;
pub mod clock;
pub mod clock_bridge;
pub(crate) mod codec;
mod duration;
pub mod engine_backend;
pub mod latency;
pub mod native;
pub mod process_loopback;
pub mod professional;
pub mod render;
pub(crate) mod resample;
pub(crate) mod servo;
pub mod shared_stream;
pub mod stats;
pub mod vad;

pub mod virtual_owner;
pub use duration::{DurationError, SessionDuration};

/// Native WASAPI/virtual-endpoint sample rate every owner in this crate
/// targets. The engine may still run at another rate; owners adapt through
/// the clock bridge and boundary resamplers.
pub(crate) const NATIVE_SAMPLE_RATE: u32 = 48_000;

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
mod owner;

#[cfg(windows)]
mod preparation;

#[cfg(windows)]
pub(crate) mod negotiation;
