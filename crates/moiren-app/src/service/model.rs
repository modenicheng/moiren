use moiren_core::protocol::ControlReply;
use std::{sync::Arc, time::Duration};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionGeneration(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLimit {
    UntilStopped,
    Seconds(u16),
}
#[derive(Debug, Clone, PartialEq)]
pub enum InputSelection {
    Physical { endpoint_id: String },
    Process { pid: u32, creation_time_100ns: u64 },
    Tone { frequency_hz: f64 },
}
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSpec {
    pub source: InputSelection,
    pub output_endpoint_id: String,
    pub limit: RunLimit,
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
}
impl SessionSpec {
    pub fn validate(&self) -> Result<(), DispatchError> {
        let source_valid = match &self.source {
            InputSelection::Physical { endpoint_id } => endpoint_valid(endpoint_id),
            InputSelection::Process { pid, .. } => *pid != 0,
            InputSelection::Tone { frequency_hz } => {
                frequency_hz.is_finite() && *frequency_hz > 0.0 && *frequency_hz < 24000.0
            }
        };
        if !source_valid
            || !endpoint_valid(&self.output_endpoint_id)
            || !valid_gain_pan(self.gain, self.pan)
            || !(1..=4096).contains(&self.max_block_frames)
            || matches!(self.limit,RunLimit::Seconds(n) if !(1..=600).contains(&n))
        {
            return Err(DispatchError::InvalidConfig);
        }
        Ok(())
    }
}
fn endpoint_valid(id: &str) -> bool {
    !id.is_empty() && !id.contains('\0')
}
pub(crate) fn valid_gain_pan(gain: f64, pan: f64) -> bool {
    gain.is_finite()
        && pan.is_finite()
        && (0.0..=1.0).contains(&gain)
        && (-1.0..=1.0).contains(&pan)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Idle,
    Starting,
    Running,
    Stopping,
    Failed,
    Exiting,
    Exited,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    Busy,
    Exiting,
    Disconnected,
    InvalidConfig,
    GenerationExhausted,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    Start(SessionSpec),
    Stop,
    RefreshCatalog,
    SetGainPan { gain: f64, pan: f64 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub endpoint_id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRow {
    pub pid: u32,
    pub creation_time_100ns: u64,
    pub name: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceCatalog {
    pub inputs: Vec<DeviceRow>,
    pub outputs: Vec<DeviceRow>,
    pub processes: Vec<ProcessRow>,
    pub default_input_endpoint_id: Option<String>,
    pub default_output_endpoint_id: Option<String>,
    pub default_roles: Vec<DefaultEndpointRow>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultEndpointRow {
    pub flow: &'static str,
    pub role: &'static str,
    pub endpoint_id: Option<String>,
}
/// The UI keeps only the latest pending pair and latest reply, never a reply history.
#[derive(Debug, Clone, Default)]
pub struct ControlSummary {
    pub accepted_pending: usize,
    pub pending_gain_pan: Option<(f64, f64)>,
    pub last_result: Option<ControlReply>,
}
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub generation: SessionGeneration,
    pub phase: SessionPhase,
    pub desired: Option<SessionSpec>,
    pub error: Option<String>,
    pub catalog: Arc<DeviceCatalog>,
    pub control: ControlSummary,
    pub elapsed: Duration,
}
