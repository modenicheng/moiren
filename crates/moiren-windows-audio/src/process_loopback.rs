//! Process-tree capture selection. Identity is resolved before owner startup and
//! checked again by the owner; a recycled PID never silently changes the source.
use crate::SessionDuration;
use crate::capture::{CaptureError, PcmArtifactConfig};
use serde::Serialize;

mod bridge;
pub use bridge::ProcessBridgeOptions;
mod measured;
pub use measured::{MeasuredAdmissionError, MeasuredProcessDelivery};
#[cfg(windows)]
pub(crate) mod activation;
#[cfg(windows)]
mod identity;
#[cfg(windows)]
mod stream;
#[cfg(windows)]
pub use identity::inspect_process;
#[cfg(all(windows, feature = "probe-tools"))]
pub(crate) use stream::{capture_format, process_loopback_stream_flags};
#[cfg(windows)]
pub use stream::{
    prepare_process_capture_with_gate, prepare_process_capture_with_gate_returning_with_abort,
    start_process_capture, start_process_capture_with_stop,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_time_100ns: u64,
    /// Basename only; reports do not persist a user's full application path.
    pub executable_name: String,
}
impl ProcessIdentity {
    pub fn same_process(&self, other: &Self) -> bool {
        self.pid == other.pid && self.creation_time_100ns == other.creation_time_100ns
    }
}
#[derive(Debug, Clone)]
pub struct ProcessLoopbackOptions {
    pub artifact: Option<PcmArtifactConfig>,
    pub bridge: ProcessBridgeOptions,
    pub latency_telemetry: crate::latency::TelemetryMode,
    pub target: ProcessIdentity,
    pub duration: SessionDuration,
}
impl ProcessLoopbackOptions {
    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.target.pid == 0 || self.target.creation_time_100ns == 0 {
            return Err(CaptureError::InvalidProcess);
        }
        if self.target.pid == std::process::id() {
            return Err(CaptureError::FeedbackTarget);
        }
        crate::capture::validate_duration(self.duration)?;
        self.bridge.prepare()?;
        Ok(())
    }
}
