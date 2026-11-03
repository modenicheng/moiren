//! Separate bounded packet/service history; sample payload never depends on it.
use serde::Serialize;

pub const RENDER_EVENT_CAPACITY: usize = 4096;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderEventKind {
    Start,
    Stop,
    Wait,
    #[default]
    Write,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderEventSample {
    pub kind: RenderEventKind,
    pub qpc_before_100ns: Option<u64>,
    pub qpc_after_100ns: Option<u64>,
    /// Actual write-side GetCurrentPadding result; unknown for other phases.
    pub padding_frames: Option<u32>,
    pub requested_frames: u64,
    /// Reserved native frames with a completed release attempt, not playback.
    pub submitted_frames: u32,
    pub release_hresult: Option<i32>,
    pub failed: bool,
    /// Only meaningful for Wait. A timeout is not an underrun verdict.
    pub ready: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderEvent {
    pub sequence: u64,
    /// Absent for zero writes and failures before an observed packet release.
    pub packet_sequence: Option<u64>,
    pub accepted_frames: u64,
    pub sample: RenderEventSample,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderTimelineSummary {
    pub capacity_events: usize,
    pub events: u64,
    pub stored_events: u64,
    pub dropped_events: u64,
    pub reader_abandoned_events: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
    pub saturated: bool,
}
