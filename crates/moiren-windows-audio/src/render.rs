//! Single-master render data path. Same-owner bridge copying at 48 kHz stereo;
//! this is not a cross-clock bridge or a sample-rate converter.
use moiren_engine::{
    boundary::{AudioReader, BridgeError},
    runtime::{Engine, RuntimeError},
};
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};
use thiserror::Error;

#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use wasapi::{
    RenderEndpoint, RenderSession, list_render_endpoints, start_render, start_render_with_stop,
};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderError {
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
    #[error("render observer already has an attached publisher")]
    ObserverInUse,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub endpoint_id: String,
    pub duration: Duration,
}
impl RenderOptions {
    /// Runs until explicitly stopped. `Duration::MAX` is the continuous sentinel;
    /// all other durations must retain the bounded 1..=600 second contract.
    pub fn continuous(endpoint_id: impl Into<String>) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            duration: Duration::MAX,
        }
    }

    pub fn validate(&self) -> Result<(), RenderError> {
        if self.endpoint_id.trim().is_empty() || self.endpoint_id.contains('\0') {
            return Err(RenderError::InvalidEndpoint);
        }
        if self.duration != Duration::MAX
            && (self.duration < Duration::from_secs(1) || self.duration > Duration::from_secs(600))
        {
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

/// A coherent observation of successful DSP work, including work before a
/// subsequent output bridge transfer fails.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RenderObservation {
    pub timeline: u64,
    pub counters: DemandCounters,
    /// Peak of the most recent nonempty render demand, including its chunks.
    pub peak_amplitude: f32,
    /// This worker successfully started its native stream. Priming DSP alone
    /// does not establish that an output device is running.
    pub stream_started: bool,
}

#[derive(Debug, Default)]
struct ObservationState {
    publisher_claimed: AtomicBool,
    revision: AtomicU64,
    timeline: AtomicU64,
    frames: AtomicU64,
    blocks: AtomicU64,
    segments: AtomicU64,
    peak_bits: AtomicU32,
    stream_started: AtomicBool,
}

/// Control-side observation without sharing the engine or consuming its output.
/// Attach once before handing the renderer to the render owner. Clones only read;
/// the single renderer publishes without locks, allocation, or destruction.
#[derive(Debug, Clone, Default)]
pub struct RenderObserver(Arc<ObservationState>);
impl RenderObserver {
    pub fn snapshot(&self) -> RenderObservation {
        loop {
            let before = self.0.revision.load(Ordering::SeqCst);
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            let observation = RenderObservation {
                timeline: self.0.timeline.load(Ordering::SeqCst),
                counters: DemandCounters {
                    frames: self.0.frames.load(Ordering::SeqCst),
                    blocks: self.0.blocks.load(Ordering::SeqCst),
                    segments: self.0.segments.load(Ordering::SeqCst),
                },
                peak_amplitude: f32::from_bits(self.0.peak_bits.load(Ordering::SeqCst)),
                stream_started: self.0.stream_started.load(Ordering::SeqCst),
            };
            if before == self.0.revision.load(Ordering::SeqCst) {
                return observation;
            }
        }
    }

    fn claim(self) -> Result<ObserverPublisher, RenderError> {
        self.0
            .publisher_claimed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| RenderError::ObserverInUse)?;
        Ok(ObserverPublisher(self))
    }
}

/// Only this non-cloneable token can publish. Claims are acquired and released
/// on the control owner; renderer publication never changes ownership.
struct ObserverPublisher(RenderObserver);
impl ObserverPublisher {
    fn publish(&self, observation: RenderObservation) {
        // A sequence protects coherence across the individual atomics. SeqCst
        // keeps the odd revision ahead of all fields and the even one after them.
        self.0.0.revision.fetch_add(1, Ordering::SeqCst);
        self.0
            .0
            .timeline
            .store(observation.timeline, Ordering::SeqCst);
        self.0
            .0
            .frames
            .store(observation.counters.frames, Ordering::SeqCst);
        self.0
            .0
            .blocks
            .store(observation.counters.blocks, Ordering::SeqCst);
        self.0
            .0
            .segments
            .store(observation.counters.segments, Ordering::SeqCst);
        self.0
            .0
            .peak_bits
            .store(observation.peak_amplitude.to_bits(), Ordering::SeqCst);
        self.0
            .0
            .stream_started
            .store(observation.stream_started, Ordering::SeqCst);
        self.0.0.revision.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for ObserverPublisher {
    fn drop(&mut self) {
        // The renderer is returned to control before destruction/extraction;
        // releasing its publication claim cannot occur inside render demand.
        self.0.0.publisher_claimed.store(false, Ordering::Release);
    }
}

pub struct DemandRenderer {
    engine: Engine<f32>,
    output: AudioReader<f32>,
    counters: DemandCounters,
    peak_amplitude: f32,
    stream_started: bool,
    observer: Option<ObserverPublisher>,
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
            peak_amplitude: 0.0,
            stream_started: false,
            observer: None,
        })
    }
    /// Attach on the control owner before starting the native worker. The
    /// initial snapshot includes any engine timeline predating preparation.
    /// Rejects an observer claimed by any renderer, retaining this renderer's
    /// existing observer and leaving both observers' snapshots unchanged.
    pub fn set_observer(&mut self, observer: RenderObserver) -> Result<(), RenderError> {
        let publisher = observer.claim()?;
        publisher.publish(self.observation());
        self.observer = Some(publisher);
        Ok(())
    }

    /// Extract on the control caller after joining the native worker. Neither
    /// the engine's processors nor the bridge allocation is destroyed on the
    /// COM/render owner during ordinary stop or backend failure. Extraction
    /// also releases the observer publication claim on this control caller.
    pub fn into_parts(self) -> (Engine<f32>, AudioReader<f32>) {
        (self.engine, self.output)
    }

    pub fn timeline(&self) -> u64 {
        self.engine.timeline()
    }
    pub fn counters(&self) -> DemandCounters {
        self.counters
    }

    fn observation(&self) -> RenderObservation {
        RenderObservation {
            timeline: self.timeline(),
            counters: self.counters,
            peak_amplitude: self.peak_amplitude,
            stream_started: self.stream_started,
        }
    }

    fn publish_observation(&self) {
        if let Some(observer) = &self.observer {
            observer.publish(self.observation());
        }
    }

    #[cfg(windows)]
    pub(crate) fn set_stream_started(&mut self, started: bool) {
        self.stream_started = started;
        self.publish_observation();
    }

    /// Successful processing performs no allocation or destruction. Empty
    /// demand does not advance the timeline; every missing tail is initialized.
    pub fn render_interleaved(&mut self, output: &mut [f32]) -> Result<DemandReport, RenderError> {
        if !output.len().is_multiple_of(CHANNELS) {
            return Err(RenderError::InvalidSamples);
        }
        output.fill(0.0);
        if !output.is_empty() {
            self.peak_amplitude = 0.0;
        }
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
            let transfer = self.output.read_interleaved(samples);
            for &sample in samples.iter() {
                self.peak_amplitude = self.peak_amplitude.max(sample.abs());
            }
            // Publish successful DSP even if the subsequent bridge operation
            // failed. The peak only examines initialized output samples.
            self.publish_observation();
            let transferred = transfer?;
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
    pub requested_seconds: f64,
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
