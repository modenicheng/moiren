//! Slint owns presentation only; backend sessions never cross this boundary.
mod bindings;
pub mod signals;
mod window;
pub use bindings::{DetailSelection, SelectionState};
pub use window::{WindowPresentation, run};
// Slint generated vtables require unsafe internals. Handwritten UI code
// remains covered by the crate-level deny; only generated code is exempt.
#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;
