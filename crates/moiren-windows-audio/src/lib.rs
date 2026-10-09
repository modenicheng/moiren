//! Windows audio probes, Shared capture/render and backend clock adaptation.

pub mod capture;
pub mod clock;
pub mod clock_bridge;
pub mod render;
pub mod stats;

#[cfg(windows)]
mod session;
#[cfg(windows)]
pub use session::StopSignal;

#[cfg(windows)]
pub mod probe;

#[cfg(windows)]
pub mod physical;

#[cfg(windows)]
pub mod swdevice;

#[cfg(windows)]
mod catalog;
#[cfg(windows)]
mod owner;
