//! Shared capture lifecycle, with backend clock adaptation before the graph.
use crate::clock_bridge::ClockBridgeError;
use crate::process_loopback::ProcessIdentity;
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;

#[cfg(windows)]
pub(crate) mod wasapi;
#[cfg(windows)]
pub use wasapi::{
    CaptureEndpoint, CaptureSession, PreparedCapture, list_capture_endpoints, start_capture,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("capture requires an explicit nonempty endpoint ID without embedded NUL")]
    InvalidEndpoint,
    #[error("capture duration must be between 1 and 600 seconds")]
    InvalidDuration,
    #[error("capture supports only native 44.1/48 kHz mono/stereo 32-bit float")]
    UnsupportedFormat,
    #[error("invalid native capture packet pointer or frame count")]
    InvalidPacket,
    #[error("native capture buffer exceeds the 8 MiB preparation budget")]
    BufferBudget,
    #[error(transparent)]
    Bridge(#[from] ClockBridgeError),
    #[error("{stage} failed (HRESULT 0x{hresult:08X})")]
    Api { stage: &'static str, hresult: i32 },
    #[error("capture worker could not start (OS error {code:?})")]
    WorkerSpawn { code: Option<i32> },
    #[error("capture worker panicked")]
    WorkerPanicked,
    #[error("capture startup channel closed before preparation completed")]
    StartupLost,
    #[error("process capture requires a nonzero PID and creation time")]
    InvalidProcess,
    #[error("target process identity changed since selection")]
    ProcessIdentityChanged,
    #[error("target process tree contains this audio host and would capture its own output")]
    FeedbackTarget,
    #[error("capture preparation was cancelled")]
    Cancelled,
    #[error("target process exited before capture preparation completed")]
    TargetExited,
    #[error("process audio activation exceeded the 10 second deadline")]
    ActivationTimeout,
}
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub endpoint_id: String,
    pub duration: Duration,
}
impl CaptureOptions {
    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.endpoint_id.trim().is_empty() || self.endpoint_id.contains('\0') {
            return Err(CaptureError::InvalidEndpoint);
        }
        validate_duration(self.duration)
    }
}
pub(crate) fn validate_duration(duration: Duration) -> Result<(), CaptureError> {
    if duration < Duration::from_secs(1) || duration > Duration::from_secs(600) {
        return Err(CaptureError::InvalidDuration);
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    Physical,
    ProcessLoopback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Completed,
    Stopped,
    TargetExited,
    Failed,
}
#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub schema_version: u32,
    pub source: CaptureSource,
    pub endpoint_id: Option<String>,
    pub process: Option<ProcessIdentity>,
    pub windows_auto_conversion: bool,
    pub activation_seconds: Option<f64>,
    pub status: CaptureStatus,
    pub requested_seconds: f64,
    pub elapsed_seconds: f64,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    pub buffer_frames: Option<u32>,
    pub stream_started: bool,
    pub stop_succeeded: bool,
    pub stop_hresult: Option<i32>,
    pub mmcss_registered: bool,
    pub mmcss_hresult: Option<i32>,
    pub audio_wakes: u64,
    pub timeout_wakes: u64,
    pub packets: u64,
    pub failure: Option<String>,
}
#[cfg(windows)]
impl CaptureReport {
    pub(crate) fn new(source: CaptureSource, duration: Duration) -> Self {
        Self {
            schema_version: 2,
            source,
            endpoint_id: None,
            process: None,
            windows_auto_conversion: source == CaptureSource::ProcessLoopback,
            activation_seconds: None,
            status: CaptureStatus::Failed,
            requested_seconds: duration.as_secs_f64(),
            elapsed_seconds: 0.0,
            sample_rate: None,
            channels: None,
            buffer_frames: None,
            stream_started: false,
            stop_succeeded: false,
            stop_hresult: None,
            mmcss_registered: false,
            mmcss_hresult: None,
            audio_wakes: 0,
            timeout_wakes: 0,
            packets: 0,
            failure: None,
        }
    }
}
