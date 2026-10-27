//! Backend-owned capture clock adaptation. The graph sees stereo f32 at its
//! prepared Engine rate; native layout and clock correction stay here.
use crate::latency::clock::QpcClock;
use rtrb::RingBuffer;
use std::sync::Arc;
use thiserror::Error;

mod ingress;
mod rate;
pub use rate::RateClock;
mod source;
mod telemetry;
mod timing;
pub use ingress::{CaptureIngress, CapturePacket};
pub use source::ClockSource;
use telemetry::Counters;
pub use telemetry::{BridgeObserver, BridgeSnapshot};
pub use timing::{QueuedFrameAge, QueuedFrameStamp};

pub const OUTPUT_SAMPLE_RATE: u32 = crate::NATIVE_SAMPLE_RATE;

#[derive(Debug, Clone, Copy)]
pub struct ClockBridgeConfig {
    pub input_sample_rate: u32,
    /// Engine output rate. Physical-only owners use 48kHz; VAD projects may
    /// select another supported processing rate after native capture SRC.
    pub output_sample_rate: u32,
    pub input_channels: usize,
    pub capacity_frames: usize,
    /// Input frames held before starting/restarting interpolation.
    pub target_fill_frames: usize,
    pub prepared_input: Option<crate::latency::PreparedInputQueue>,
    /// Discard older queued frames once at each prime to bound startup latency.
    /// With prepared watermarks, also recover running high-water backlog.
    /// Disable when consuming a finite offline recording from its first frame.
    pub trim_on_prime: bool,
    /// Physical clients expose a frame clock; some virtual process clients
    /// return zero for every device position. Native discontinuity flags remain
    /// authoritative even when position-gap inference is disabled.
    pub detect_position_gaps: bool,
    pub max_correction_ppm: f64,
    /// Ring sample/metadata storage budget, excluding Arc/ring bookkeeping.
    pub byte_budget: usize,
    /// Optional age limit for live native input. This is deliberately an
    /// explicit owner-local policy; offline FIFO consumers leave it unset.
    pub age_limit_100ns: Option<u64>,
}
impl Default for ClockBridgeConfig {
    fn default() -> Self {
        Self {
            input_sample_rate: crate::NATIVE_SAMPLE_RATE,
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            input_channels: 2,
            capacity_frames: 8192,
            target_fill_frames: 2048,
            prepared_input: None,
            trim_on_prime: true,
            detect_position_gaps: true,
            max_correction_ppm: 2000.0,
            byte_budget: 8 * 1024 * 1024,
            age_limit_100ns: None,
        }
    }
}
impl ClockBridgeConfig {
    pub fn with_prepared_input(
        input_sample_rate: u32,
        input_channels: usize,
        input: crate::latency::PreparedInputQueue,
    ) -> Self {
        Self {
            input_sample_rate,
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            input_channels,
            capacity_frames: input.capacity_frames as usize,
            target_fill_frames: input.prime_frames as usize,
            prepared_input: Some(input),
            ..Self::default()
        }
    }
    pub fn with_engine_input(
        input_sample_rate: u32,
        output_sample_rate: u32,
        input_channels: usize,
        input: crate::latency::PreparedInputQueue,
    ) -> Self {
        Self {
            input_sample_rate,
            output_sample_rate,
            input_channels,
            capacity_frames: input.capacity_frames as usize,
            target_fill_frames: input.prime_frames as usize,
            prepared_input: Some(input),
            ..Self::default()
        }
    }
    pub(super) fn servo_reference(self) -> usize {
        self.prepared_input
            .map_or(self.target_fill_frames, |input| {
                input.servo_reference_frames as usize
            })
    }
    fn prepared_input_valid(self) -> bool {
        self.prepared_input.is_none_or(|input| {
            input.capacity_frames as usize == self.capacity_frames
                && input.prime_frames as usize == self.target_fill_frames
                && input.low_water_frames >= 2
                && input.low_water_frames <= input.prime_frames
                && input.low_water_frames <= input.servo_reference_frames
                && input.prime_frames < input.high_water_frames
                && input.servo_reference_frames < input.high_water_frames
                && input.high_water_frames <= input.capacity_frames
                && input.modeled_minimum_frames.is_none_or(|minimum| {
                    minimum >= 2
                        && minimum <= input.prime_frames
                        && minimum <= input.servo_reference_frames
                })
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClockBridgeError {
    #[error(
        "capture bridge requires supported mono/stereo input and Engine output rates, a 2..capacity target, and finite 0..2000 ppm correction"
    )]
    InvalidConfig,
    #[error("capture bridge storage exceeds its preparation budget")]
    BudgetExceeded,
    #[error("capture packet length does not match complete native f32 frames")]
    InvalidPacket,
    #[error("clock source output must contain complete stereo frames")]
    InvalidOutput,
}
#[derive(Clone, Copy)]
struct Frame {
    samples: [f32; 2],
    generation: u64,
    packet_stamp: Option<timing::PacketStamp>,
    packet_offset_frames: u64,
}

/// Non-RT preparation. Every endpoint has one owner; all allocation and final
/// destruction must occur outside streaming, just like the engine sample bridge.
pub fn capture_bridge(
    config: ClockBridgeConfig,
) -> Result<(CaptureIngress, ClockSource, BridgeObserver), ClockBridgeError> {
    capture_bridge_with_clock(config, None)
}

pub(crate) fn capture_bridge_with_clock(
    config: ClockBridgeConfig,
    age_clock: Option<QpcClock>,
) -> Result<(CaptureIngress, ClockSource, BridgeObserver), ClockBridgeError> {
    if !moiren_engine::audio_config::SAMPLE_RATES.contains(&config.input_sample_rate)
        || !moiren_engine::audio_config::SAMPLE_RATES.contains(&config.output_sample_rate)
        || !(1..=2).contains(&config.input_channels)
        || config.target_fill_frames < 2
        || config.target_fill_frames >= config.capacity_frames
        || !config.prepared_input_valid()
        || config.age_limit_100ns == Some(0)
        || !config.max_correction_ppm.is_finite()
        || !(0.0..=2000.0).contains(&config.max_correction_ppm)
    {
        return Err(ClockBridgeError::InvalidConfig);
    }
    config
        .capacity_frames
        .checked_mul(size_of::<Frame>())
        .filter(|&n| n <= isize::MAX as usize && n <= config.byte_budget)
        .ok_or(ClockBridgeError::BudgetExceeded)?;
    let counters = Arc::new(Counters::new(config.capacity_frames));
    let (tx, rx) = RingBuffer::new(config.capacity_frames);
    Ok((
        CaptureIngress::new(tx, config, Arc::clone(&counters)),
        ClockSource::new(rx, config, Arc::clone(&counters), age_clock),
        BridgeObserver(counters),
    ))
}
