//! Backend-owned capture clock adaptation. The graph sees only stereo f32 at
//! 48 kHz; native packet layout, buffering and clock correction stay here.
use rtrb::RingBuffer;
use std::sync::Arc;
use thiserror::Error;

mod ingress;
mod source;
mod telemetry;
pub use ingress::{CaptureIngress, CapturePacket};
pub use source::ClockSource;
use telemetry::Counters;
pub use telemetry::{BridgeObserver, BridgeSnapshot};

pub const OUTPUT_SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone, Copy)]
pub struct ClockBridgeConfig {
    pub input_sample_rate: u32,
    pub input_channels: usize,
    pub capacity_frames: usize,
    /// Input frames held before starting/restarting interpolation.
    pub target_fill_frames: usize,
    /// Discard older queued frames once at each prime to bound startup latency.
    /// Disable when consuming a finite offline recording from its first frame.
    pub trim_on_prime: bool,
    /// Physical clients expose a frame clock; some virtual process clients
    /// return zero for every device position. Native discontinuity flags remain
    /// authoritative even when position-gap inference is disabled.
    pub detect_position_gaps: bool,
    pub max_correction_ppm: f64,
    /// Ring sample/metadata storage budget, excluding Arc/ring bookkeeping.
    pub byte_budget: usize,
}
impl Default for ClockBridgeConfig {
    fn default() -> Self {
        Self {
            input_sample_rate: 48_000,
            input_channels: 2,
            capacity_frames: 8192,
            target_fill_frames: 2048,
            trim_on_prime: true,
            detect_position_gaps: true,
            max_correction_ppm: 2000.0,
            byte_budget: 8 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClockBridgeError {
    #[error(
        "capture bridge requires native 44.1/48 kHz mono/stereo, a 2..capacity target, and finite 0..2000 ppm correction"
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
}

/// Non-RT preparation. Every endpoint has one owner; all allocation and final
/// destruction must occur outside streaming, just like the engine sample bridge.
pub fn capture_bridge(
    config: ClockBridgeConfig,
) -> Result<(CaptureIngress, ClockSource, BridgeObserver), ClockBridgeError> {
    if ![44_100, 48_000].contains(&config.input_sample_rate)
        || !(1..=2).contains(&config.input_channels)
        || config.target_fill_frames < 2
        || config.target_fill_frames >= config.capacity_frames
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
        ClockSource::new(rx, config, Arc::clone(&counters)),
        BridgeObserver(counters),
    ))
}
