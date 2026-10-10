//! Non-RT contracts shared by native adapters and deterministic backend tests.
use super::{DeviceCatalog, SessionSpec};
use moiren_core::{graph::NodeId, protocol::ControlReply};
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_windows_audio::render::TimelineSnapshot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{stage}: {reason}")]
pub struct BackendError {
    pub stage: &'static str,
    pub reason: String,
}
impl BackendError {
    pub fn new(stage: &'static str, reason: impl ToString) -> Self {
        Self {
            stage,
            reason: reason.to_string(),
        }
    }
}
/// Cancellation is persistent; clones are safe before preparation has returned.
#[derive(Clone)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
    #[cfg(windows)]
    stop: moiren_windows_audio::StopSignal,
}
impl Cancellation {
    pub fn new() -> Result<Self, BackendError> {
        Ok(Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            stop: moiren_windows_audio::StopSignal::new()
                .map_err(|e| BackendError::new("stop signal", e))?,
        })
    }
    pub fn cancel(&self) -> Result<(), BackendError> {
        self.cancelled.store(true, Ordering::Release);
        #[cfg(windows)]
        self.stop
            .request_stop()
            .map_err(|e| BackendError::new("stop signal", e))?;
        Ok(())
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    #[cfg(windows)]
    pub(crate) fn stop_signal(&self) -> moiren_windows_audio::StopSignal {
        self.stop.clone()
    }
}
pub trait BackendAdapter: Send + Sync + 'static {
    fn prepare(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError>;
    fn catalog(&self) -> Result<DeviceCatalog, BackendError>;
}
pub trait PreparedSession: Send {
    fn activate(&self) -> Result<(), BackendError>;
    fn into_active(self: Box<Self>) -> Box<dyn ActiveSession>;
    fn cancel(&self) -> Result<(), BackendError>;
    /// Blocking join and final control retirement belong to a worker.
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartedState {
    Starting,
    Running,
    Failed,
}
pub trait ActiveSession: Send {
    fn request_stop(&self) -> Result<(), BackendError>;
    fn is_finished(&self) -> bool;
    fn poll_started(&self) -> StartedState;
    fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)>;
    fn timeline(&self) -> Option<TimelineSnapshot> {
        None
    }
    fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        None
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    Completed,
    Stopped,
    TargetExited,
    Failed,
}
#[derive(Debug)]
pub struct SessionReport {
    pub status: SessionEnd,
    pub error: Option<String>,
    /// At most the 64 accepted requests that were unresolved at handoff.
    pub control_replies: Vec<ControlReply>,
}
impl SessionReport {
    pub fn stopped() -> Self {
        Self {
            status: SessionEnd::Stopped,
            error: None,
            control_replies: Vec::with_capacity(64),
        }
    }
}
