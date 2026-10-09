//! Physical Shared capture, with backend clock adaptation before the graph.
use crate::clock_bridge::ClockBridgeError;
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;

#[cfg(windows)]
mod wasapi;
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
        if self.duration < Duration::from_secs(1) || self.duration > Duration::from_secs(600) {
            return Err(CaptureError::InvalidDuration);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Completed,
    Stopped,
    Failed,
}
#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub schema_version: u32,
    pub endpoint_id: String,
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
