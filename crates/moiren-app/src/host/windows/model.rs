use super::*;
use serde::Serialize;

/// Pinned process identity is deliberately part of the selection. A restart
/// must never silently resolve an old PID to a different process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSelection {
    Physical { endpoint_id: String },
    Process { identity: ProcessIdentity },
}

#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub output_endpoint_id: String,
    pub sources: Vec<CaptureSelection>,
    pub config: HostConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Staged,
    Running,
    Disabled,
    Stopped,
    TargetExited,
    Failed,
    Removed,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceDiagnostic {
    pub id: u64,
    pub selection: CaptureSelection,
    pub status: SourceStatus,
    pub enabled: bool,
    pub available: bool,
    pub bridge: BridgeSnapshot,
    pub failure: Option<String>,
    pub last_start_failure: Option<String>,
    pub report: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionRuntime {
    pub timeline: u64,
    pub peak_amplitude: f32,
    pub rendered_frames: u64,
    pub rendered_blocks: u64,
    pub rendered_segments: u64,
    pub stream_started: bool,
    pub active_revision: u64,
    pub desired_revision: u64,
    pub pending: bool,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub schema_version: u32,
    pub status: SessionStatus,
    pub output_endpoint_id: String,
    pub failure: Option<String>,
    pub runtime: SessionRuntime,
    pub sources: Vec<SourceDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct SessionReport {
    pub snapshot: SessionSnapshot,
    pub render: Option<RenderReport>,
    /// Includes terminal replies reclaimed during shutdown, without an extra
    /// audible render block. Match parameter receipts by request_id.
    pub events: Vec<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Capture(#[from] moiren_windows_audio::capture::CaptureError),
    #[error(transparent)]
    Render(#[from] moiren_windows_audio::render::RenderError),
    #[error("session is not running")]
    NotRunning,
    #[error("unknown session source {0:?}")]
    UnknownSource(SourceId),
    #[error("an exited process requires an explicit new pinned process selection")]
    ProcessRestartNeedsSelection,
    #[error("render startup failed: {0}")]
    Startup(String),
}

pub(super) struct CaptureOwner {
    pub selection: CaptureSelection,
    pub session: Option<CaptureSession>,
    pub observer: BridgeObserver,
    pub gate: SourceGate,
    pub status: SourceStatus,
    pub enabled: bool,
    pub report: Option<CaptureReport>,
    pub failure: Option<String>,
    pub last_start_failure: Option<String>,
}
impl CaptureOwner {
    pub fn diagnostic(&self, id: SourceId) -> SourceDiagnostic {
        SourceDiagnostic {
            id: id.0,
            selection: self.selection.clone(),
            status: self.status,
            enabled: self.enabled,
            available: self.gate.is_available(),
            bridge: self.observer.snapshot(),
            failure: self.failure.clone(),
            last_start_failure: self.last_start_failure.clone(),
            report: self
                .report
                .as_ref()
                .map(|r| serde_json::to_value(r).expect("capture report serialization")),
        }
    }
    pub fn join(&mut self, explicit_stop: bool) {
        self.gate.set_available(false);
        if self.session.is_none()
            && explicit_stop
            && matches!(
                self.status,
                SourceStatus::Staged | SourceStatus::Running | SourceStatus::Disabled
            )
        {
            self.status = SourceStatus::Stopped;
        }
        if let Some(worker) = self.session.take() {
            if explicit_stop && let Err(error) = worker.request_stop() {
                self.failure = Some(error.to_string());
            }
            match worker.join() {
                Ok(report) => {
                    self.status = match report.status {
                        CaptureStatus::TargetExited => SourceStatus::TargetExited,
                        CaptureStatus::Failed => SourceStatus::Failed,
                        _ => SourceStatus::Stopped,
                    };
                    if report.failure.is_some() {
                        self.failure = report.failure.clone();
                    }
                    self.report = Some(report);
                }
                Err(error) => {
                    self.status = SourceStatus::Failed;
                    self.failure = Some(error.to_string());
                }
            }
        }
    }
}
