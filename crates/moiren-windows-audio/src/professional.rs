//! Configurable native audio. Preparation and reclamation stay on control;
//! native interfaces never leave the thread that opened them.
pub(crate) mod codec;
pub mod native;
pub(crate) mod resample;

#[cfg(windows)]
pub mod asio;
#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod bridge;
mod observations;
#[cfg(windows)]
mod owner;
#[cfg(any(windows, test))]
mod recovery;
#[cfg(windows)]
pub mod wasapi;

#[cfg(windows)]
pub use backend::ProfessionalBackend;
pub use observations::{
    ProfessionalObservationSnapshot, ProfessionalObservations, ProfessionalStreamObservation,
};
