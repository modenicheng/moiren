//! Windows audio probes, Shared capture/render and backend clock adaptation.

pub mod capture;
pub mod clock;
pub mod clock_bridge;
mod duration;
pub mod process_loopback;
pub mod render;
pub mod stats;
pub use duration::{DurationError, SessionDuration};

#[cfg(windows)]
mod session;
#[cfg(windows)]
pub use session::{Activation, ActivationGate, GateError, StopSignal};

#[cfg(windows)]
pub mod probe;

#[cfg(windows)]
pub mod physical;

#[cfg(windows)]
pub mod swdevice;

#[cfg(windows)]
mod catalog;
#[cfg(windows)]
pub use catalog::{
    CatalogSnapshot, DefaultEndpoint, EndpointSnapshot, owned_snapshot as catalog_snapshot,
};
#[cfg(windows)]
mod owner;
