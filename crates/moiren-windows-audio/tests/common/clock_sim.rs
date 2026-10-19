//! Mechanics shared by the three `clock_*` long-run simulation kernels
//! (`tests/clock_control.rs`, `tests/clock_startup.rs`, `tests/clock_faults.rs`).
//!
//! The event loops, budget models and fault injections stay per test (they are
//! the scenarios under test); the duplicated capture-side scheduling math
//! lives here once.
#![allow(dead_code)] // shared across test binaries; each compiles the subset it uses

/// Native output rate all three kernels render at.
pub const OUTPUT_RATE: u32 = 48_000;

/// Capture-side interval, in output frames, between two capture callbacks of
/// `capture_period` frames captured at `input_rate` drifting by `drift_ppm`.
pub fn capture_interval(capture_period: f64, input_rate: u32, drift_ppm: f64) -> f64 {
    capture_period * f64::from(OUTPUT_RATE)
        / f64::from(input_rate)
        / (1.0 + drift_ppm / 1_000_000.0)
}

/// QPC stamp (100 ns units) for the capture callback whose nominal position is
/// `nominal_capture_at`, given the scheduling `capture_interval`.
pub fn capture_stamp(nominal_capture_at: f64, capture_interval: f64) -> u64 {
    ((nominal_capture_at - capture_interval).max(0.0) * 10_000_000.0 / f64::from(OUTPUT_RATE))
        as u64
        + 1
}

/// Fills `payload` with an interleaved stereo ramp of frame indices starting
/// at `position`, so downstream tests can identify sample provenance.
pub fn fill_ramp(payload: &mut [u8], position: u64) {
    for (frame, samples) in payload.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let sample = (position + frame as u64) as f32;
        samples[..4].copy_from_slice(&sample.to_le_bytes());
        samples[4..].copy_from_slice(&sample.to_le_bytes());
    }
}

/// Deterministic burst-jitter pattern: 8 of every 17 events arrive `frames`
/// late, the rest on time.
pub fn burst_delay(event: u64, frames: u32) -> u32 {
    if event % 17 < 8 { frames } else { 0 }
}
