//! Windows audio probes, Shared capture/render and backend clock adaptation.

pub mod capture;
pub mod clock;
pub mod clock_bridge;
mod duration;
pub mod engine_backend;
pub mod latency;
pub mod process_loopback;
pub mod professional;
pub mod render;
pub mod shared_stream;
pub mod stats;
pub use duration::{DurationError, SessionDuration};

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
