//! Windows audio probes and a minimal engine-driven Shared render backend.

pub mod clock;
pub mod render;
pub mod stats;

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
