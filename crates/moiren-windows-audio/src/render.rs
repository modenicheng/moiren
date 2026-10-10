//! Single-master render data path. Same-owner bridge copying at 48 kHz stereo;
//! this is not a cross-clock bridge or a sample-rate converter.
use crate::SessionDuration;
use moiren_engine::{
    boundary::{AudioReader, BridgeError},
    runtime::{Engine, RuntimeError},
};
use serde::Serialize;
use thiserror::Error;

#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use wasapi::{
    PreparedRenderSession, RenderEndpoint, RenderOwnerExit, RenderSession, list_render_endpoints,
    start_render, start_render_prepared, start_render_prepared_with_gate, start_render_with_stop,
};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderError {
    #[cfg(windows)]
    #[error(transparent)]
    Gate(#[from] crate::GateError),
    #[error("render startup channel closed before preparation completed")]
    StartupLost,
    #[error("only native 48 kHz stereo 32-bit float is supported by this render slice")]
    UnsupportedFormat,
    #[error("render requires an explicit nonempty endpoint ID without embedded NUL")]
    InvalidEndpoint,
    #[error("render duration must be between 1 and 600 seconds")]
    InvalidDuration,
    #[error("render samples must contain complete stereo frames")]
    InvalidSamples,
    #[error("output bridge must hold at least one engine maximum block")]
    BridgeCapacity,
    #[error("native render staging exceeds the 8 MiB preparation budget")]
    StagingBudget,
    #[error("device capacity is zero or padding exceeds capacity")]
    InvalidPadding,
    #[error("output bridge transferred {actual} frames, expected {expected}")]
    IncompleteTransfer { expected: usize, actual: usize },
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error("{stage} failed (HRESULT 0x{hresult:08X})")]
    Api { stage: &'static str, hresult: i32 },
    #[error("render worker could not start (OS error {code:?})")]
    WorkerSpawn { code: Option<i32> },
    #[error("render worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub endpoint_id: String,
    pub duration: SessionDuration,
}
impl RenderOptions {
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.endpoint_id.trim().is_empty() || self.endpoint_id.contains('\0') {
            return Err(RenderError::InvalidEndpoint);
        }
        if self.duration.validate().is_err() {
            return Err(RenderError::InvalidDuration);
        }
        Ok(())
    }
}

pub fn writable_frames(capacity: u32, padding: u32) -> Result<u32, RenderError> {
    if capacity == 0 {
        return Err(RenderError::InvalidPadding);
    }
    capacity
        .checked_sub(padding)
        .ok_or(RenderError::InvalidPadding)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DemandReport {
    pub frames: usize,
    pub blocks: usize,
    pub segments: usize,
}

/// Cumulative successful DSP work since preparation, including blocks whose
/// later bridge transfer failed. Independent of the engine's prior timeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DemandCounters {
    pub frames: u64,
    pub blocks: u64,
    pub segments: u64,
}

pub struct DemandRenderer {
    engine: Engine<f32>,
    output: AudioReader<f32>,
    counters: DemandCounters,
}
impl DemandRenderer {
    /// Non-RT preparation. The reader must be the matching, initially drained
    /// graph output bridge; no other thread may consume it.
    pub fn new(engine: Engine<f32>, output: AudioReader<f32>) -> Result<Self, RenderError> {
        if engine.config().processing_sr != f64::from(SAMPLE_RATE)
            || output.channel_count() != CHANNELS
        {
            return Err(RenderError::UnsupportedFormat);
        }
        if output.capacity_frames() < engine.config().max_block_frames {
            return Err(RenderError::BridgeCapacity);
        }
        Ok(Self {
            engine,
            output,
            counters: DemandCounters::default(),
        })
    }
    pub fn timeline(&self) -> u64 {
        self.engine.timeline()
    }
    pub fn counters(&self) -> DemandCounters {
        self.counters
    }
    /// After the owner has joined, poll ControlPort replies and retry until
    /// zero before dropping either endpoint. This never renders another block.
    pub fn retire_controls(&mut self) -> usize {
        self.engine.retire_controls()
    }

    /// Successful processing performs no allocation or destruction. Empty
    /// demand does not advance the timeline; every missing tail is initialized.
    pub fn render_interleaved(&mut self, output: &mut [f32]) -> Result<DemandReport, RenderError> {
        if !output.len().is_multiple_of(CHANNELS) {
            return Err(RenderError::InvalidSamples);
        }
        output.fill(0.0);
        let mut report = DemandReport {
            frames: output.len() / CHANNELS,
            ..DemandReport::default()
        };
        let block_samples = self.engine.config().max_block_frames * CHANNELS;
        for samples in output.chunks_mut(block_samples) {
            let frames = samples.len() / CHANNELS;
            let rendered = self.engine.render(frames)?;
            self.counters.frames += frames as u64;
            self.counters.blocks += 1;
            self.counters.segments += rendered.segments as u64;
            let transferred = self.output.read_interleaved(samples)?;
            if transferred.transferred_frames != frames {
                return Err(RenderError::IncompleteTransfer {
                    expected: frames,
                    actual: transferred.transferred_frames,
                });
            }
            report.blocks += 1;
            report.segments += rendered.segments;
        }
        Ok(report)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderStatus {
    Completed,
    Stopped,
    Failed,
}

#[derive(Debug, Default, Serialize)]
pub struct RenderStats {
    pub primed_frames: u32,
    pub submitted_frames: u64,
    pub processed_frames: u64,
    pub dsp_blocks: u64,
    pub dsp_segments: u64,
    pub audio_wakes: u64,
    pub timeout_wakes: u64,
    pub zero_demand_wakes: u64,
    /// A diagnostic only, not definitive underrun evidence.
    pub empty_padding_wakes: u64,
    pub min_writable_frames: Option<u32>,
    pub max_writable_frames: u32,
    pub bridge_shortfall_frames: u64,
    pub nonzero_samples: u64,
    pub peak_amplitude: f32,
}

#[derive(Debug, Serialize)]
pub struct RenderReport {
    pub schema_version: u32,
    pub endpoint_id: String,
    pub status: RenderStatus,
    pub requested_seconds: Option<f64>,
    pub elapsed_seconds: f64,
    pub sample_rate: u32,
    pub channels: usize,
    pub native_mix_sample_rate: Option<u32>,
    pub native_mix_channels: Option<u16>,
    pub native_mix_container_bits: Option<u16>,
    pub format_supported: bool,
    pub buffer_frames: Option<u32>,
    pub engine_period_frames: Option<u32>,
    pub period_hresult: Option<i32>,
    pub stream_latency_100ns: Option<i64>,
    pub mmcss_registered: bool,
    pub mmcss_hresult: Option<i32>,
    pub own_session_ducking_opt_out: bool,
    pub ducking_hresult: Option<i32>,
    pub stop_succeeded: bool,
    pub stream_started: bool,
    pub stop_hresult: Option<i32>,
    pub stats: RenderStats,
    pub failure: Option<String>,
}
