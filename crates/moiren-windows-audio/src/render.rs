//! Single-master render data path. Same-owner bridge copying at 48 kHz stereo;
//! this is not a cross-clock bridge or a sample-rate converter.
use crate::SessionDuration;
use moiren_core::protocol::ProcessorId;
use moiren_engine::{
    boundary::{AudioReader, BridgeError},
    runtime::{Engine, PlanSwapError, RuntimeError},
};
use serde::Serialize;
use thiserror::Error;

mod diagnostics;
mod fade;
#[cfg(windows)]
mod source;
pub use diagnostics::{
    RENDER_EVENT_CAPACITY, RenderCallback, RenderCallbackHistory, RenderClockSample,
    RenderDiagnostics, RenderDiagnosticsSnapshot, RenderEvent, RenderPaddingObserver,
    RenderPaddingOwner, RenderPaddingSample, RenderPaddingSnapshot,
};
pub use fade::{FadeError, RenderFadeControl};
#[cfg(windows)]
pub(crate) use source::{FollowerRenderer, NativeRenderer, SampleRenderer};

#[cfg(windows)]
pub(crate) mod wasapi;
#[cfg(windows)]
pub use wasapi::{
    NativeRenderInfo, PreparedNativeRender, PreparedRenderSession, RenderAttachFailure,
    RenderEndpoint, RenderOwnerExit, RenderSession, list_render_endpoints,
    prepare_render_native_with_gate, start_render, start_render_prepared,
    start_render_prepared_with_gate, start_render_with_stop,
};

pub const SAMPLE_RATE: u32 = crate::NATIVE_SAMPLE_RATE;
pub const CHANNELS: usize = 2;

mod timeline;
pub use timeline::{ManagedTimelineSnapshot, TimelineObserver, TimelineSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderError {
    #[cfg(windows)]
    #[error(transparent)]
    Gate(#[from] crate::GateError),
    #[error("render startup channel closed before preparation completed")]
    StartupLost,
    #[error("render requires a valid PCM/float mix descriptor and a 48 kHz stereo f32 engine")]
    UnsupportedFormat,
    #[error("render requires an explicit nonempty endpoint ID without embedded NUL")]
    InvalidEndpoint,
    #[error("demand renderer does not support plan switching")]
    PlanSwitchingUnsupported,
    #[error(transparent)]
    PlanSwap(#[from] PlanSwapError),
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
    #[error("render preparation was cancelled")]
    Cancelled,
    #[error(transparent)]
    SharedPeriod(#[from] crate::shared_stream::SharedPeriodError),
    #[error(transparent)]
    LatencyBudget(#[from] crate::latency::LatencyBudgetError),
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub queue: crate::latency::RenderQueuePolicy,
    pub latency_telemetry: crate::latency::TelemetryMode,
    pub shared_period: crate::shared_stream::SharedPeriodPolicy,
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
        self.shared_period.validate()?;
        self.queue.validate()?;
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
    timeline: TimelineObserver,
    fade: fade::FadeEnvelope,
}

pub struct RendererPreparationFailure {
    pub error: RenderError,
    pub engine: Engine<f32>,
    pub output: AudioReader<f32>,
}
impl DemandRenderer {
    /// Non-RT preparation. The reader must be the matching, initially drained
    /// graph output bridge; no other thread may consume it.
    pub fn new(engine: Engine<f32>, output: AudioReader<f32>) -> Result<Self, RenderError> {
        if engine.plan_switching_enabled() {
            return Err(RenderError::PlanSwitchingUnsupported);
        }
        Self::prepare(engine, output, 0)
    }

    /// The reader must match the initial sink, as for `new`. Every subsequent
    /// activation must transfer that sink instance, including its sole writer.
    pub fn new_managed(
        engine: Engine<f32>,
        output: AudioReader<f32>,
        sink: ProcessorId,
    ) -> Result<Self, RenderError> {
        let generation = engine.epoch();
        Self::new_managed_with_generation(engine, output, sink, generation)
    }

    pub fn new_managed_with_generation(
        engine: Engine<f32>,
        output: AudioReader<f32>,
        sink: ProcessorId,
        owner_generation: u64,
    ) -> Result<Self, RenderError> {
        Self::new_managed_returning(engine, output, sink, owner_generation)
            .map_err(|failure| failure.error)
    }

    pub fn new_managed_returning(
        mut engine: Engine<f32>,
        output: AudioReader<f32>,
        sink: ProcessorId,
        owner_generation: u64,
    ) -> Result<Self, Box<RendererPreparationFailure>> {
        let validation = Self::validate(&engine, &output)
            .and_then(|()| engine.require_sink_reuse(sink).map_err(Into::into));
        if let Err(error) = validation {
            return Err(Box::new(RendererPreparationFailure {
                error,
                engine,
                output,
            }));
        }
        Ok(Self::prepared(engine, output, owner_generation))
    }

    /// Non-RT attachment for native fan-out. All readers remain connected to
    /// their original sink instances through every compatible activation.
    pub fn new_managed_outputs_returning(
        mut engine: Engine<f32>,
        output: AudioReader<f32>,
        sinks: &[ProcessorId],
        owner_generation: u64,
    ) -> Result<Self, Box<RendererPreparationFailure>> {
        let validation = Self::validate(&engine, &output).and_then(|()| {
            if sinks.is_empty()
                || engine
                    .plan_snapshot()
                    .sink_processors()
                    .any(|sink| !sinks.contains(&sink))
            {
                return Err(PlanSwapError::MissingReuse.into());
            }
            if sinks
                .iter()
                .enumerate()
                .any(|(index, sink)| sinks[..index].contains(sink))
            {
                return Err(PlanSwapError::DuplicateReuse.into());
            }
            for &sink in sinks {
                engine.require_sink_reuse(sink)?;
            }
            Ok(())
        });
        if let Err(error) = validation {
            return Err(Box::new(RendererPreparationFailure {
                error,
                engine,
                output,
            }));
        }
        Ok(Self::prepared(engine, output, owner_generation))
    }

    fn prepare(
        engine: Engine<f32>,
        output: AudioReader<f32>,
        owner_generation: u64,
    ) -> Result<Self, RenderError> {
        Self::validate(&engine, &output)?;
        Ok(Self::prepared(engine, output, owner_generation))
    }

    fn validate(engine: &Engine<f32>, output: &AudioReader<f32>) -> Result<(), RenderError> {
        if engine.config().processing_sr != f64::from(SAMPLE_RATE)
            || output.channel_count() != CHANNELS
        {
            return Err(RenderError::UnsupportedFormat);
        }
        if output.capacity_frames() < engine.config().max_block_frames {
            return Err(RenderError::BridgeCapacity);
        }
        Ok(())
    }

    fn prepared(engine: Engine<f32>, output: AudioReader<f32>, owner_generation: u64) -> Self {
        let timeline = TimelineObserver::new(owner_generation, &engine);
        Self {
            engine,
            output,
            counters: DemandCounters::default(),
            timeline,
            fade: fade::FadeEnvelope::new(),
        }
    }
    pub fn timeline(&self) -> u64 {
        self.engine.timeline()
    }
    pub fn fade_control(&self) -> RenderFadeControl {
        self.fade.control()
    }
    /// Call only after the last rendered buffer was successfully submitted.
    pub fn acknowledge_submission(&self) {
        self.fade.acknowledge_submission();
    }
    pub fn timeline_observer(&self) -> TimelineObserver {
        self.timeline.clone()
    }
    pub fn counters(&self) -> DemandCounters {
        self.counters
    }
    /// Call only after the native owner has stopped and returned this renderer.
    pub fn into_parts(self) -> (Engine<f32>, AudioReader<f32>) {
        (self.engine, self.output)
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
            let rendered = self.engine.render(frames);
            self.timeline.publish(&self.engine);
            let rendered = rendered?;
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
        self.fade.apply(output);
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
    pub queue: Option<crate::latency::PreparedRenderQueue>,
    pub schema_version: u32,
    pub latency: crate::latency::LatencySnapshot,
    pub diagnostics: RenderDiagnosticsSnapshot,
    /// Retain the allocation outside the native loop's unwind scope. Only the
    /// finalized snapshot is serialized, and the joining control owner frees it.
    #[cfg(windows)]
    #[serde(skip)]
    diagnostics_recorder: Option<RenderDiagnostics>,
    pub shared_period: crate::shared_stream::SharedPeriodReport,
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
    /// Raw mix-rate engine period. SharedPeriodReport::actual_period_frames
    /// is the corresponding client-rate period used for queue admission.
    pub engine_period_frames: Option<u32>,
    pub period_hresult: Option<i32>,
    pub stream_latency_100ns: Option<i64>,
    pub clock_frequency: Option<u64>,
    /// Optional clock preparation failure; never rejects supported audio IO.
    pub clock_hresult: Option<i32>,
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

#[cfg(windows)]
impl RenderReport {
    fn prepare_diagnostics(&mut self, target_frames: u32) {
        let budget = (u64::from(target_frames) * 10_000_000).div_ceil(u64::from(SAMPLE_RATE));
        self.diagnostics.wake_budget_100ns = budget;
        if let Some(recorder) = &mut self.diagnostics_recorder {
            recorder.set_wake_budget(budget);
        }
    }

    fn record_diagnostics(&mut self, callback: RenderCallback) {
        if let Some(recorder) = &mut self.diagnostics_recorder {
            recorder.record(callback);
        }
    }

    fn finish_diagnostics(&mut self) {
        if let Some(recorder) = self.diagnostics_recorder.take() {
            self.diagnostics = recorder.finish();
        }
    }
}
