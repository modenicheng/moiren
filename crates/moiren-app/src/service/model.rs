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
impl InputSelection {
    /// The source-domain half of [`SessionSpec::validate`]; graph candidates
    /// use it directly instead of constructing a throwaway spec.
    pub fn validate(&self) -> Result<(), DispatchError> {
        let source_valid = match self {
            InputSelection::Physical { endpoint_id } => endpoint_valid(endpoint_id),
            InputSelection::Process { pid, .. } => *pid != 0,
            InputSelection::Tone { frequency_hz } => {
                frequency_hz.is_finite() && *frequency_hz > 0.0 && *frequency_hz < 24000.0
            }
        };
        source_valid
            .then_some(())
            .ok_or(DispatchError::InvalidConfig)
    }
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
        self.source.validate()?;
        if !endpoint_valid(&self.output_endpoint_id)
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
    RequestIdExhausted,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    LoadProject(crate::project::ProjectDocument),
    SetExportAuthorizations(Vec<super::ExportAuthorization>),
    SetLatencyProfile(super::AudioLatencyProfile),
    SetAudioSettings(super::AudioSettings),
    SetTransportLatency(super::TransportLatencySettings),
    /// Select current native evidence and rebuild; never mutates a live queue.
    ApplyMeasuredProcessBridge(super::MeasuredProcessRequest),
    /// Require a sealed supply/demand window from the service's pinned owner.
    ApplyWindowedProcessBridge {
        request: super::MeasuredProcessRequest,
        policy: super::ProcessWindowPolicy,
    },
    #[cfg(windows)]
    ApplyEvaluatedProcessBridge(super::EvaluatedProcessCandidate),
    EditGraph(super::GraphEdit),
    /// Restore the candidate content recorded before the last user edit.
    UndoGraph,
    /// Re-apply an undone edit; invalidated by any new edit.
    RedoGraph,
    ReplaceGraph {
        graph: moiren_core::graph::LogicalGraph,
        nodes: Vec<super::GraphNodeState>,
    },
    ApplyGraph,
    Start(SessionSpec),
    Stop,
    RefreshCatalog,
    SetGainPan {
        gain: f64,
        pan: f64,
    },
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
    /// Exact MMDevice IDs indexed by VAD export identity; ownership is never
    /// inferred from a display name, and uncertain retirements stay excluded.
    pub vad_endpoints: std::collections::BTreeMap<moiren_engine::node::VirtualEndpointId, String>,
    /// Native ASIO drivers use stable IDs accepted by the professional backend.
    pub asio_drivers: Vec<DeviceRow>,
    /// ASIO discovery can fail independently of usable WASAPI endpoints.
    pub asio_error: Option<String>,
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlSummary {
    pub pending_node_parameters: usize,
    pub accepted_pending: usize,
    pub pending_gain_pan: Option<(f64, f64)>,
    pub last_result: Option<ControlReply>,
}
/// Successful mailbox enqueue order; zero means no ordinary request consumed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId(pub u64);
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    /// Last reaped host, kept separately from live-generation observations.
    /// A joined service thread alone does not establish successful cleanup.
    pub last_cleanup: Option<SessionCleanupObservation>,
    #[cfg(windows)]
    pub process_trace_status: Option<super::ProcessTraceCollectionStatus>,
    pub loaded_project: Option<Arc<crate::project::ProjectDocument>>,
    pub exports: Vec<super::ExportStatus>,
    pub export_plan: super::ExportReconcilePlan,
    #[cfg(windows)]
    pub native_observations: Option<NativeSessionObservations>,
    #[cfg(windows)]
    pub professional_observations: Option<ProfessionalSessionObservations>,
    pub latency_profile: super::AudioLatencyProfile,
    pub active_latency_profile: Option<super::AudioLatencyProfile>,
    pub audio_settings: super::AudioSettings,
    pub active_audio_settings: Option<super::AudioSettings>,
    pub transport_latency: super::TransportLatencySettings,
    /// Requested bounds owned by the Running generation. Effective native
    /// queues are reported separately in its diagnostics.
    pub active_transport_latency: Option<super::TransportLatencySettings>,
    pub measured_process: Option<super::MeasuredProcessSummary>,
    pub active_measured_process: Option<super::MeasuredProcessSummary>,
    pub graph: Arc<super::GraphSnapshot>,
    /// Consumption is published with the phase produced by those requests.
    pub consumed_request_id: RequestId,
    pub generation: SessionGeneration,
    pub phase: SessionPhase,
    pub desired: Option<SessionSpec>,
    pub error: Option<String>,
    pub catalog: Arc<DeviceCatalog>,
    /// Refresh failure retains the last successful rows without changing the
    /// session phase/error. A successful refresh clears this owned diagnostic.
    pub catalog_error: Option<String>,
    pub control: ControlSummary,
    pub signal_topics: super::SignalTopics,
    /// Whether the active session feeds its levels into the tone topic
    /// instead of the physical monitor pair. The meter bridge resolves this
    /// from the running host graph and publishes it as a fact, so every
    /// consumer (meter subscriptions today) reads one answer instead of
    /// re-deriving "is this a software source" from its own input.
    pub tone_session: bool,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub struct SessionCleanupObservation {
    pub generation: SessionGeneration,
    pub state: moiren_engine::host::HostSnapshot,
    #[cfg(windows)]
    pub process_trace: Option<Arc<super::ProcessTraceSessionEvidence>>,
    #[cfg(windows)]
    pub final_observations: Option<Arc<JoinedSessionObservations>>,
}

/// Read from retained counter handles only after the corresponding host joined.
/// Missing handles stay unknown; these are not the throttled UI snapshots.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
pub struct JoinedSessionObservations {
    pub graph_revision: u64,
    pub host_revision: u64,
    pub native: Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>,
    pub backend: Option<moiren_windows_audio::engine_backend::ProfessionalObservationSnapshot>,
}
// Retained trace evidence is immutable and Arc-backed; comparing identity
// avoids walking its bounded packet history on every idle publication.
impl PartialEq for SessionCleanupObservation {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(windows)]
        {
            match (&self.final_observations, &other.final_observations) {
                (None, None) => {}
                (Some(left), Some(right)) if Arc::ptr_eq(left, right) => {}
                _ => return false,
            }
            match (&self.process_trace, &other.process_trace) {
                (None, None) => {}
                (Some(left), Some(right)) if Arc::ptr_eq(left, right) => {}
                _ => return false,
            }
        }
        self.generation == other.generation && self.state == other.state
    }
}
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
pub struct NativeSessionObservations {
    pub generation: SessionGeneration,
    pub observation: moiren_windows_audio::engine_backend::NativeObservationSnapshot,
}

/// Publish-side change detection: while a session runs the only fields that
/// advance on their own are the quantized clock and the tick-batched
/// observations, so equality holds between real events. The loaded project
/// Arc is swapped only on load, so pointer equality short-circuits a
/// potentially large deep comparison; a same-content reload at worst costs
/// one redundant publish. The graph Arc is rebuilt per snapshot, so it
/// compares by value (small fixed topology).
impl PartialEq for AppSnapshot {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(windows)]
        {
            if self.professional_observations != other.professional_observations
                || self.native_observations != other.native_observations
                || self.process_trace_status != other.process_trace_status
            {
                return false;
            }
        }
        fn same_arc<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
            match (left, right) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
        }
        same_arc(&self.loaded_project, &other.loaded_project)
            && self.last_cleanup == other.last_cleanup
            && (Arc::ptr_eq(&self.catalog, &other.catalog) || self.catalog == other.catalog)
            && self.exports == other.exports
            && self.export_plan == other.export_plan
            && self.latency_profile == other.latency_profile
            && self.active_latency_profile == other.active_latency_profile
            && self.audio_settings == other.audio_settings
            && self.active_audio_settings == other.active_audio_settings
            && self.transport_latency == other.transport_latency
            && self.active_transport_latency == other.active_transport_latency
            && self.measured_process == other.measured_process
            && self.active_measured_process == other.active_measured_process
            && self.graph == other.graph
            && self.consumed_request_id == other.consumed_request_id
            && self.generation == other.generation
            && self.phase == other.phase
            && self.desired == other.desired
            && self.error == other.error
            && self.catalog_error == other.catalog_error
            && self.control == other.control
            && self.signal_topics == other.signal_topics
            && self.tone_session == other.tone_session
            && self.elapsed == other.elapsed
    }
}
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
pub struct ProfessionalSessionObservations {
    pub generation: SessionGeneration,
    pub observation: moiren_windows_audio::engine_backend::ProfessionalObservationSnapshot,
}
