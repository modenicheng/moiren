//! Serialized probe records are independent of the thread-owned COM services.
use crate::{
    catalog::{ApiFailure, CatalogSnapshot, FormatSnapshot},
    clock::{ClockPoint, ClockSummary, DeviceClockSummary},
    probe::PacketRecord,
    stats::CaptureSummary,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct DemandRecord {
    pub arrival_ms: f64,
    pub padding_frames: u32,
    pub writable_frames: u32,
}

#[derive(Default, Serialize)]
pub struct RenderSummary {
    pub primed_frames: u32,
    pub submitted_frames: u64,
    pub zero_demand_wakes: u64,
    /// An empty buffer observation is a diagnostic, not proof of an underrun.
    pub empty_padding_wakes: u64,
    pub min_writable_frames: Option<u32>,
    pub max_writable_frames: u32,
}

#[derive(Serialize)]
pub struct EndpointReport {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub flow: &'static str,
    pub status: &'static str,
    pub last_stage: &'static str,
    pub elapsed_seconds: f64,
    pub format: Option<FormatSnapshot>,
    pub buffer_frames: Option<u32>,
    pub stream_latency_100ns: Option<i64>,
    pub engine_period_frames: Option<u32>,
    pub category: &'static str,
    pub own_session_ducking_opt_out: bool,
    pub mmcss_registered: bool,
    pub stop_succeeded: bool,
    pub audio_wakes: u64,
    pub timeout_wakes: u64,
    pub frequency_units_per_second: Option<u64>,
    pub clock: ClockSummary,
    /// Capture frame/QPC pairs, normalized by stream sample rate, not GetFrequency.
    pub capture_packet_clock: Option<ClockSummary>,
    pub comparison_clock_source: &'static str,
    pub clock_points: Vec<ClockPoint>,
    /// Optional IAudioClock2 raw positions are device frames, not client bytes.
    pub device_clock: Option<DeviceClockSummary>,
    pub device_clock_points: Vec<ClockPoint>,
    pub capture: Option<CaptureSummary>,
    pub capture_packets: Vec<PacketRecord>,
    pub render: Option<RenderSummary>,
    pub render_demands: Vec<DemandRecord>,
    pub metadata_dropped: u64,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize)]
pub struct RelativeClock {
    pub left_endpoint: String,
    pub right_endpoint: String,
    pub left_faster_ppm: Option<f64>,
}

#[derive(Serialize)]
pub struct PhysicalReport {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub requested_seconds: u32,
    pub before: CatalogSnapshot,
    pub endpoints: Vec<EndpointReport>,
    pub common_qpc_window_100ns: Option<(u64, u64)>,
    pub relative_clocks: Vec<RelativeClock>,
    pub after: Option<CatalogSnapshot>,
    pub observed_state_changes: Vec<String>,
    pub errors: Vec<ApiFailure>,
}

impl EndpointReport {
    pub(super) fn new(endpoint_id: String, name: Option<String>) -> Self {
        Self {
            endpoint_id,
            name,
            flow: "unknown",
            status: "initializing",
            last_stage: "CoInitializeEx",
            elapsed_seconds: 0.0,
            format: None,
            buffer_frames: None,
            stream_latency_100ns: None,
            engine_period_frames: None,
            category: "Other",
            own_session_ducking_opt_out: false,
            mmcss_registered: false,
            stop_succeeded: false,
            audio_wakes: 0,
            timeout_wakes: 0,
            frequency_units_per_second: None,
            clock: ClockSummary::default(),
            capture_packet_clock: None,
            comparison_clock_source: "unavailable",
            clock_points: Vec::new(),
            device_clock: None,
            device_clock_points: Vec::new(),
            capture: None,
            capture_packets: Vec::new(),
            render: None,
            render_demands: Vec::new(),
            metadata_dropped: 0,
            errors: Vec::new(),
        }
    }
}
