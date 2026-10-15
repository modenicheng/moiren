//! Portable project and preference persistence, called on the control side.
//!
//! Documents contain candidate configuration and presentation state. Loading a
//! project never starts audio or changes the service's active session.

mod graph;
mod model;
mod storage;

pub use graph::*;
pub use model::*;
pub use storage::*;

#[cfg(test)]
mod tests;
