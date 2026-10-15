//! Non-RT contracts shared by native adapters and deterministic backend tests.
use super::{DeviceCatalog, SessionSpec, SignalPreparation};
use crate::monitor::{MonitorFaults, MonitorLatencyConfig, MonitorObservation};
use moiren_core::{graph::NodeId, protocol::ControlReply};
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

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
    /// Adapters must explicitly opt into arbitrary topology. The compatibility
    /// path is valid only for the original fixed chain, never silent fallback.
    fn prepare_graph(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
        signals: &mut SignalPreparation,
        graph: &super::GraphPreparation,
        latency: Option<MonitorLatencyConfig>,
        fade_in_frames: u32,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        if graph.graph
            != crate::session_graph::SessionGraph::new()
                .map_err(|e| BackendError::new("graph", e))?
                .graph
        {
            return Err(BackendError::new(
                "graph",
                "adapter does not support edited topology",
            ));
        }
        if let Some(latency) = latency {
            self.prepare_with_monitor_latency(spec, cancellation, signals, latency, fade_in_frames)
        } else {
            self.prepare_with_signals(spec, cancellation, signals)
        }
    }
    fn prepare(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError>;
    fn prepare_with_signals(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
        _signals: &mut SignalPreparation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        self.prepare(spec, cancellation)
    }
    fn catalog(&self) -> Result<DeviceCatalog, BackendError>;
    fn prepare_with_monitor_latency(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
        signals: &mut SignalPreparation,
        latency: MonitorLatencyConfig,
        fade_in_frames: u32,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        if latency != MonitorLatencyConfig::default() || fade_in_frames != 0 {
            return Err(BackendError::new(
                "monitor latency",
                "adapter does not support this policy",
            ));
        }
        self.prepare_with_signals(spec, cancellation, signals)
    }
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
/// Adapter observation used to correlate application controls with the active
/// frame, timeline epoch and plan revision, independent of the native backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineSnapshot {
    pub frame: u64,
    pub epoch: u64,
    pub revision: u64,
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
    fn monitor_faults(&self) -> Option<MonitorFaults> {
        None
    }
    fn monitor_observation(&self) -> Option<MonitorObservation> {
        None
    }
    fn request_monitor_fade_out(&self, _frames: u32) -> Result<(), BackendError> {
        Err(BackendError::new(
            "monitor fade",
            "adapter does not support output fade",
        ))
    }
    fn monitor_fade_out_submitted(&self) -> bool {
        false
    }
    fn monitor_queue_drain_duration(&self) -> Duration {
        Duration::ZERO
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
