//! Process-tree capture selection. Identity is resolved before owner startup and
//! checked again by the owner; a recycled PID never silently changes the source.
use crate::capture::CaptureError;
use serde::Serialize;
use std::time::Duration;

#[cfg(windows)]
pub(crate) mod activation;
#[cfg(windows)]
mod identity;
#[cfg(windows)]
mod stream;
#[cfg(windows)]
pub use identity::{inspect_process, list_processes};
#[cfg(windows)]
pub(crate) use stream::capture_format;
#[cfg(windows)]
pub use stream::{start_process_capture, start_process_capture_with_stop};

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
    pub target: ProcessIdentity,
    pub duration: Duration,
}
impl ProcessLoopbackOptions {
    /// Runs until explicitly stopped. `Duration::MAX` is the continuous sentinel;
    /// all other durations retain the bounded 1..=600 second contract.
    pub fn continuous(target: ProcessIdentity) -> Self {
        Self {
            target,
            duration: Duration::MAX,
        }
    }

    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.target.pid == 0 || self.target.creation_time_100ns == 0 {
            return Err(CaptureError::InvalidProcess);
        }
        if self.target.pid == std::process::id() {
            return Err(CaptureError::FeedbackTarget);
        }
        crate::capture::validate_duration(self.duration)
    }
}
