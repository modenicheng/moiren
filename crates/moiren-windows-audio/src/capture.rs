//! Shared capture lifecycle, with backend clock adaptation before the graph.
use crate::SessionDuration;
use crate::clock_bridge::ClockBridgeError;
use crate::process_loopback::ProcessIdentity;
use serde::Serialize;
use thiserror::Error;

mod artifact;
pub use artifact::{
    PcmArtifact, PcmArtifactConfig, PcmArtifactError, PcmArtifactMetadata, PcmRecorder,
};

#[cfg(windows)]
pub(crate) mod wasapi;
#[cfg(windows)]
pub use wasapi::{
    CaptureEndpoint, CaptureSession, NativeCaptureInfo, PreparedCapture, PreparedNativeCapture,
    list_capture_endpoints, prepare_capture_native_with_gate, prepare_capture_with_gate,
    start_capture, start_capture_with_stop,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[cfg(windows)]
    #[error(transparent)]
    Gate(#[from] crate::GateError),
    #[error("capture requires an explicit nonempty endpoint ID without embedded NUL")]
    InvalidEndpoint,
    #[error("capture duration must be between 1 and 600 seconds")]
    InvalidDuration,
    #[error(
        "invalid or unsupported capture mix descriptor; Shared compatibility requires a valid PCM/float format"
    )]
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
    #[error("PCM artifact preparation failed: {0}")]
    PcmArtifact(#[from] PcmArtifactError),
    #[error(transparent)]
    SharedPeriod(#[from] crate::shared_stream::SharedPeriodError),
    #[error(transparent)]
    LatencyBudget(#[from] crate::latency::LatencyBudgetError),
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
    /// Optional bounded PCM evidence retained by the capture owner. The
    /// backing storage is allocated before Start and only receives packet
    /// copies while the WASAPI lease is valid.
    pub artifact: Option<PcmArtifactConfig>,
    pub latency_telemetry: crate::latency::TelemetryMode,
    pub shared_period: crate::shared_stream::SharedPeriodPolicy,
    pub endpoint_id: String,
    pub duration: SessionDuration,
}
impl CaptureOptions {
    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.endpoint_id.trim().is_empty() || self.endpoint_id.contains('\0') {
            return Err(CaptureError::InvalidEndpoint);
        }
        validate_duration(self.duration)?;
        self.shared_period.validate()?;
        Ok(())
    }
}
pub(crate) fn validate_duration(duration: SessionDuration) -> Result<(), CaptureError> {
    if duration.validate().is_err() {
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
    #[serde(skip)]
    pub artifact: Option<PcmArtifact>,
    pub artifact_metadata: Option<PcmArtifactMetadata>,
    #[serde(skip)]
    pub(crate) artifact_recorder: Option<PcmRecorder>,
    pub delivery_bound: Option<crate::latency::transport::InputDeliveryTiming>,
    pub delivery: crate::latency::InputDeliverySnapshot,
    pub input_queue: Option<crate::latency::PreparedInputQueue>,
    pub schema_version: u32,
    pub latency: crate::latency::LatencySnapshot,
    pub shared_period: crate::shared_stream::SharedPeriodReport,
    pub stream_latency_100ns: Option<i64>,
    pub source: CaptureSource,
    pub endpoint_id: Option<String>,
    pub process: Option<ProcessIdentity>,
    pub windows_auto_conversion: bool,
    pub activation_seconds: Option<f64>,
    pub status: CaptureStatus,
    pub requested_seconds: Option<f64>,
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
    pub(crate) fn new(source: CaptureSource, duration: SessionDuration) -> Self {
        Self {
            schema_version: 8,
            artifact: None,
            artifact_metadata: None,
            artifact_recorder: None,
            delivery_bound: None,
            delivery: Default::default(),
            input_queue: None,
            latency: crate::latency::LatencyRecorder::new(crate::latency::TelemetryMode::Disabled)
                .snapshot(),
            shared_period: crate::shared_stream::SharedPeriodReport::default(),
            stream_latency_100ns: None,
            source,
            endpoint_id: None,
            process: None,
            windows_auto_conversion: source == CaptureSource::ProcessLoopback,
            activation_seconds: None,
            status: CaptureStatus::Failed,
            requested_seconds: duration.requested_seconds(),
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

    pub(crate) fn attach_artifact(&mut self, artifact: PcmArtifact) {
        self.artifact_metadata = Some(artifact.metadata());
        self.artifact = Some(artifact);
    }

    pub(crate) fn prepare_artifact(
        &mut self,
        config: Option<PcmArtifactConfig>,
        sample_rate: u32,
        channels: usize,
    ) -> Result<(), CaptureError> {
        self.artifact_recorder = config
            .map(|config| PcmRecorder::new(config, sample_rate, channels))
            .transpose()?;
        Ok(())
    }

    pub(crate) fn finish_artifact(&mut self) {
        if let Some(recorder) = self.artifact_recorder.take() {
            self.attach_artifact(recorder.finish());
        }
    }
}
