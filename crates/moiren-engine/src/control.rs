//! IPC worker -> one control owner -> bounded SPSC -> RT table -> applied replies.
//! Blocking I/O/decoding/coalescing belongs to the worker, never these RT methods.

mod channel;
mod ramp;
mod runtime;
mod schema;
mod view;

pub use channel::{ControlPort, parameter_channel};
pub use ramp::FloatRamp;
pub use runtime::ParameterRuntime;
pub use schema::{ControlError, ParamDomain, ParamSpec};
pub(crate) use view::ParameterBindings;
pub use view::ProcessParameters;
