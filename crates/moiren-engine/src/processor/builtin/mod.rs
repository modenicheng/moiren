//! Built-in realtime DSP processors.
mod gain;
mod pan;
mod sum;

pub use gain::Gain;
pub use pan::Pan;
/// Bus and Sum share the same prepared mixing kernel. Dynamic logical input
/// editing stays in core; runtime inputs are fixed by the prepared IO bindings.
pub use sum::Sum as Bus;
pub use sum::Sum;
