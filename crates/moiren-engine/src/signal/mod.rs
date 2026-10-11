//! Scoped realtime publication capabilities owned by the compiled engine.

mod plan;
mod process;

pub(crate) use plan::PreparedSignalPlan;
pub use plan::SignalPlanError;
pub use process::ProcessSignals;
