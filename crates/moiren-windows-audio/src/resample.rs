//! Streaming windowed-sinc SRC with fixed storage and precomputed phase kernels.
//! No allocation, trigonometry or reclamation occurs while moving samples.
use std::f64::consts::PI;

use crate::{codec::finite, native::StereoFrame};
use thiserror::Error;

const TAPS: usize = 32;
const HALF: usize = TAPS / 2;
const PHASES: usize = 256;

/// Whole-wake input envelope for the nominal physical master only. Pump uses
/// exact SRC demand, so Engine segment ceilings add no rounding reserve.
/// The last output needs D-1 ratio steps, leaving at least one step of numerical
/// margin in ceil(D * ratio) + HALF. Under the professional rate/demand limits,
/// accumulated binary64 error is < 3e-10 frames, versus a minimum step > 0.11.
/// Initial lookahead needs HALF; normalized retained phase adds no block tail.
/// A follower's corrected ratio must not use this nominal bound.
#[cfg(any(windows, test))]
pub(crate) fn nominal_master_input_bound(
    input_rate: u32,
    output_rate: u32,
    output_frames: u32,
) -> Option<u32> {
    if !moiren_engine::audio_config::SAMPLE_RATES.contains(&input_rate)
        || !(8_000..=384_000).contains(&output_rate)
        || !(1..=65_536).contains(&output_frames)
    {
        return None;
    }
    let frames = (u64::from(output_frames) * u64::from(input_rate))
        .div_ceil(u64::from(output_rate))
        .checked_add(HALF as u64)?;
    u32::try_from(frames).ok()
}

/// Accepted magnitude of a drift correction, in ppm. The follower servos clip
/// their desired correction to the same bound enforced here.
pub(super) const CORRECTION_PPM_LIMIT: f64 = 2000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("SRC rates must be 8..384 kHz, and capacity must be 64..1048576 frames")]
pub struct ResampleError;

/// Reset-scoped FIR coordinates, not native timestamps or physical onset.
/// Split the next center so a large integer origin never absorbs its phase.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct SrcCursor {
    /// Increments on every actual FIR reset, not on every transport epoch.
    pub reset_generation: u64,
    /// Successful FIR pushes since reset, including playback silence-fill.
    /// This is not a native source packet frontier or a loss-free frame count.
    pub input_frames: u64,
    /// Successful FIR outputs since reset, even when later muted or dropped.
    pub output_frames: u64,
    /// Center of the next FIR output in this reset's pushed-input coordinates.
    pub next_input_frame: u64,
    pub fraction: f64,
    pub step: f64,
    /// Sticky across resets: exhausted counters cannot regain provenance.
    pub saturated: bool,
}

pub struct StereoResampler {
    frames: Box<[StereoFrame]>,
    kernels: Box<[[f64; TAPS]]>,
    head: usize,
    len: usize,
    position: f64,
    nominal_step: f64,
    step: f64,
    reset_generation: u64,
    input_frames: u64,
    output_frames: u64,
    discarded_frames: u64,
    counters_saturated: bool,
}

impl StereoResampler {
    /// Non-RT preparation. The filter needs 16 input frames of lookahead.
    pub fn new(input_rate: u32, output_rate: u32, capacity: usize) -> Result<Self, ResampleError> {
        if !(8_000..=384_000).contains(&input_rate)
            || !(8_000..=384_000).contains(&output_rate)
            || !(64..=1_048_576).contains(&capacity)
        {
            return Err(ResampleError);
        }
        let cutoff = (f64::from(output_rate) / f64::from(input_rate)).min(1.0) * 0.94;
        let mut kernels = vec![[0.0; TAPS]; PHASES + 1].into_boxed_slice();
        for (phase, kernel) in kernels.iter_mut().enumerate() {
            let fraction = phase as f64 / PHASES as f64;
            let mut sum = 0.0;
            for (tap, coefficient) in kernel.iter_mut().enumerate() {
                let distance = tap as f64 - (HALF - 1) as f64 - fraction;
                let x = PI * distance * cutoff;
                let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
                let window = 0.42
                    + 0.5 * (PI * distance / HALF as f64).cos()
                    + 0.08 * (2.0 * PI * distance / HALF as f64).cos();
                *coefficient = cutoff * sinc * window.max(0.0);
                sum += *coefficient;
            }
            for coefficient in kernel {
                *coefficient /= sum;
            }
        }
        let nominal_step = f64::from(input_rate) / f64::from(output_rate);
        Ok(Self {
            frames: vec![[0.0; 2]; capacity].into_boxed_slice(),
            kernels,
            head: 0,
            len: HALF,
            position: HALF as f64,
            nominal_step,
            step: nominal_step,
            reset_generation: 0,
            input_frames: 0,
            output_frames: 0,
            discarded_frames: 0,
            counters_saturated: false,
        })
    }

    /// Upper bound for all heap storage allocated by `new` at this capacity.
    pub fn storage_bytes(capacity: usize) -> Option<usize> {
        let frames = capacity.checked_mul(std::mem::size_of::<StereoFrame>())?;
        let kernels = (PHASES + 1)
            .checked_mul(TAPS)?
            .checked_mul(std::mem::size_of::<f64>())?;
        frames.checked_add(kernels)
    }

    /// Clear stream history without releasing or reallocating prepared storage.
    pub fn reset(&mut self) {
        add_counter(&mut self.reset_generation, 1, &mut self.counters_saturated);
        self.input_frames = 0;
        self.output_frames = 0;
        self.discarded_frames = 0;
        self.head = 0;
        self.len = HALF;
        self.position = HALF as f64;
        self.step = self.nominal_step;
        self.frames.fill([0.0; 2]);
    }

    /// Fixed scalar observation; no clock, allocation or sample traversal.
    pub fn cursor(&self) -> SrcCursor {
        let whole = self.position.floor() as u64;
        let next = whole
            .checked_sub(HALF as u64)
            .and_then(|local| self.discarded_frames.checked_add(local));
        SrcCursor {
            reset_generation: self.reset_generation,
            input_frames: self.input_frames,
            output_frames: self.output_frames,
            next_input_frame: next.unwrap_or(u64::MAX),
            fraction: self.position - whole as f64,
            step: self.step,
            saturated: self.counters_saturated || next.is_none(),
        }
    }

    #[cfg(any(windows, test))]
    pub fn queued_frames(&self) -> usize {
        self.len.saturating_sub(self.position.floor() as usize)
    }
    #[cfg(any(windows, test))]
    pub fn free_frames(&self) -> usize {
        self.frames.len() - self.len
    }

    /// Minimum new input required to make the requested number of outputs
    /// available, including the FIR's retained lookahead.
    pub fn input_frames_needed(&self, output_frames: usize) -> Option<usize> {
        if output_frames == 0 {
            return Some(0);
        }
        let last_position = self.position + (output_frames - 1) as f64 * self.step;
        let required_len = (last_position.floor() as usize).checked_add(HALF + 1)?;
        Some(required_len.saturating_sub(self.len))
    }

    /// Positive correction consumes source faster to reduce queue occupancy.
    pub fn set_correction_ppm(&mut self, ppm: f64) {
        self.step = self.nominal_step
            * (1.0 + finite(ppm).clamp(-CORRECTION_PPM_LIMIT, CORRECTION_PPM_LIMIT) * 1e-6);
    }

    pub fn push_frame(&mut self, frame: StereoFrame) -> bool {
        if self.len == self.frames.len() {
            return false;
        }
        let index = (self.head + self.len) % self.frames.len();
        self.frames[index] = [finite(frame[0]), finite(frame[1])];
        self.len += 1;
        add_counter(&mut self.input_frames, 1, &mut self.counters_saturated);
        true
    }

    /// Bulk push exists for the test harnesses' packetized feeds; production
    /// paths refill frame-by-frame via [`StereoResampler::push_frame`].
    #[cfg(test)]
    pub fn push(&mut self, input: &[StereoFrame]) -> usize {
        let frames = input.len().min(self.free_frames());
        for &frame in &input[..frames] {
            self.push_frame(frame);
        }
        frames
    }

    pub fn next_frame(&mut self) -> Option<StereoFrame> {
        let center = self.position.floor() as usize;
        if center + HALF >= self.len {
            return None;
        }
        let phase = (self.position - center as f64) * PHASES as f64;
        let index = phase.floor() as usize;
        let blend = phase - index as f64;
        let first = center - (HALF - 1);
        let mut output = [0.0; 2];
        for tap in 0..TAPS {
            let coefficient = self.kernels[index][tap]
                + blend * (self.kernels[index + 1][tap] - self.kernels[index][tap]);
            let sample = self.frames[(self.head + first + tap) % self.frames.len()];
            output[0] += sample[0] * coefficient;
            output[1] += sample[1] * coefficient;
        }
        self.position += self.step;
        // Retain the FIR history and fractional position across every packet.
        let discard = (self.position.floor() as usize)
            .saturating_sub(HALF)
            .min(self.len.saturating_sub(HALF));
        self.head = (self.head + discard) % self.frames.len();
        self.len -= discard;
        self.position -= discard as f64;
        add_counter(&mut self.output_frames, 1, &mut self.counters_saturated);
        add_counter(
            &mut self.discarded_frames,
            discard as u64,
            &mut self.counters_saturated,
        );
        Some(output)
    }

    // Native owners use bulk output; portable virtual boundaries pull frames.
    // Keep the bulk path available to the cross-platform shadow RT harness.
    #[cfg(any(windows, test))]
    pub fn produce(&mut self, output: &mut [StereoFrame]) -> usize {
        let mut count = 0;
        for frame in output {
            let Some(sample) = self.next_frame() else {
                break;
            };
            *frame = sample;
            count += 1;
        }
        count
    }
}

fn add_counter(counter: &mut u64, delta: u64, saturated: &mut bool) {
    *counter = counter.checked_add(delta).unwrap_or_else(|| {
        *saturated = true;
        u64::MAX
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_master_bound_rejects_invalid_or_unrepresentable_demands() {
        assert_eq!(nominal_master_input_bound(0, 48_000, 1), None);
        assert_eq!(nominal_master_input_bound(48_000, 7_999, 1), None);
        assert_eq!(nominal_master_input_bound(384_001, 48_000, 1), None);
        assert_eq!(nominal_master_input_bound(48_000, 48_000, 0), None);
        assert_eq!(nominal_master_input_bound(48_000, 48_000, u32::MAX), None);
        assert_eq!(nominal_master_input_bound(384_000, 8_000, u32::MAX), None);
        assert_eq!(nominal_master_input_bound(48_000, 48_000, 65_537), None);
        assert_eq!(nominal_master_input_bound(8_000, 48_000, 1), None);
        assert_eq!(nominal_master_input_bound(48_000, 48_000, 512), Some(528));
        assert_eq!(nominal_master_input_bound(48_000, 96_000, 128), Some(80));
    }

    #[test]
    fn nominal_master_bound_covers_reachable_fractional_phase_and_initial_lookahead() {
        for input_rate in moiren_engine::audio_config::SAMPLE_RATES {
            for output_rate in [8_000, 44_100, 48_000, 88_200, 96_000, 384_000] {
                let mut src = StereoResampler::new(input_rate, output_rate, 64).unwrap();
                for round in 0..24 {
                    for demand in [1, 2, 7, 32, 64, 128, 336, 512, 738, 4096] {
                        let bound =
                            nominal_master_input_bound(input_rate, output_rate, demand).unwrap();
                        let before = src.cursor();
                        let mut produced = 0;
                        while produced < demand {
                            if src.next_frame().is_some() {
                                produced += 1;
                            } else {
                                assert!(src.push_frame([0.125, -0.25]));
                            }
                        }
                        let consumed = src.cursor().input_frames - before.input_frames;
                        assert!(
                            consumed <= u64::from(bound),
                            "{input_rate}/{output_rate}, demand {demand}, round {round}: {consumed} > {bound}"
                        );
                        if input_rate == output_rate {
                            let expected =
                                u64::from(demand) + if before.output_frames == 0 { 16 } else { 0 };
                            assert_eq!(consumed, expected);
                        }
                    }
                }
                src.reset();
                let initial = src.input_frames_needed(4096).unwrap();
                assert!(
                    initial
                        <= nominal_master_input_bound(input_rate, output_rate, 4096).unwrap()
                            as usize
                );
                // Exercise the full admitted native-demand limit rather than
                // extending the small-packet result by nominal arithmetic.
                let before = src.cursor().input_frames;
                let mut produced = 0;
                while produced < 65_536 {
                    if src.next_frame().is_some() {
                        produced += 1;
                    } else {
                        assert!(src.push_frame([0.125, -0.25]));
                    }
                }
                assert!(
                    src.cursor().input_frames - before
                        <= u64::from(
                            nominal_master_input_bound(input_rate, output_rate, 65_536).unwrap()
                        )
                );
            }
        }
    }

    #[test]
    fn src_cursor_tracks_fractional_consumption_and_reset_without_fitting_an_origin() {
        let mut src = StereoResampler::new(48_000, 48_000, 128).unwrap();
        let initial = src.cursor();
        assert_eq!(initial.input_frames, 0);
        assert_eq!(initial.output_frames, 0);
        assert_eq!(initial.next_input_frame, 0);
        assert_eq!(initial.fraction, 0.0);
        assert_eq!(initial.step, 1.0);
        for ppm in [2000.0, -2000.0] {
            src.set_correction_ppm(ppm);
            let before = src.cursor();
            assert_eq!(src.push(&[[0.25; 2]; 32]), 32);
            for _ in 0..8 {
                src.next_frame().unwrap();
            }
            let after = src.cursor();
            assert_eq!(after.input_frames - before.input_frames, 32);
            assert_eq!(after.output_frames - before.output_frames, 8);
            let consumed = (after.next_input_frame - before.next_input_frame) as f64
                + after.fraction
                - before.fraction;
            assert!((consumed - 8.0 * before.step).abs() < 1e-12);
            assert!(!after.saturated);
        }
        src.reset();
        let reset = src.cursor();
        assert_eq!(reset.reset_generation, initial.reset_generation + 1);
        assert_eq!(reset.input_frames, 0);
        assert_eq!(reset.output_frames, 0);
        assert_eq!(reset.next_input_frame, 0);
        assert_eq!(reset.fraction, 0.0);
        assert_eq!(reset.step, 1.0);
    }

    #[test]
    fn src_cursor_large_split_origins_and_counter_exhaustion_never_alias_zero() {
        let mut src = StereoResampler::new(48_000, 48_000, 128).unwrap();
        src.discarded_frames = (1_u64 << 54) + 3;
        src.position = HALF as f64 + 0.125;
        assert_eq!(src.cursor().next_input_frame, (1_u64 << 54) + 3);
        assert_eq!(src.cursor().fraction, 0.125);
        src.reset();
        src.input_frames = u64::MAX;
        assert!(src.push_frame([0.0; 2]));
        assert_eq!(src.cursor().input_frames, u64::MAX);
        assert!(src.cursor().saturated);
        src.reset();
        assert!(src.cursor().saturated);
        src.reset_generation = u64::MAX;
        src.reset();
        assert_eq!(src.cursor().reset_generation, u64::MAX);
        assert!(src.cursor().saturated);
        let mut src = StereoResampler::new(48_000, 48_000, 128).unwrap();
        src.push(&[[0.25; 2]; 32]);
        src.output_frames = u64::MAX;
        src.next_frame().unwrap();
        assert_eq!(src.cursor().output_frames, u64::MAX);
        assert!(src.cursor().saturated);
        let mut src = StereoResampler::new(48_000, 48_000, 128).unwrap();
        src.discarded_frames = u64::MAX;
        assert_eq!(src.cursor().next_input_frame, u64::MAX);
        assert!(
            !src.cursor().saturated,
            "subtract local FIR prefix before adding origin"
        );
        src.position += 1.0;
        assert!(src.cursor().saturated);
    }

    fn convert(input: &[StereoFrame], from: u32, to: u32, packet: usize) -> Vec<StereoFrame> {
        let mut src = StereoResampler::new(from, to, 512).unwrap();
        let mut output = Vec::new();
        let mut offset = 0;
        while offset < input.len() {
            let end = (offset + packet).min(input.len());
            offset += src.push(&input[offset..end]);
            while let Some(frame) = src.next_frame() {
                output.push(frame);
            }
        }
        output
    }

    #[test]
    fn packet_boundaries_preserve_fractional_phase_and_stereo() {
        let input: Vec<_> = (0..4000)
            .map(|i| {
                let s = (i as f64 * 0.03).sin();
                [s, -s]
            })
            .collect();
        for from in [44100, 48000, 88200, 96000] {
            for to in [44100, 48000, 88200, 96000] {
                let a = convert(&input, from, to, 127);
                let b = convert(&input, from, to, 1);
                assert_eq!(a, b);
                assert!(
                    (a.len() as f64 - (input.len() - HALF) as f64 * to as f64 / from as f64).abs()
                        < 2.0
                );
                assert!(a.iter().all(|frame| (frame[0] + frame[1]).abs() < 1e-12));
            }
        }
    }

    #[test]
    fn downsampling_suppresses_above_nyquist_and_preserves_dc() {
        let high: Vec<_> = (0..6000)
            .map(|i| [(2.0 * PI * 30000.0 * i as f64 / 96000.0).sin(); 2])
            .collect();
        let output = convert(&high, 96000, 44100, 73);
        let rms = (output[100..].iter().map(|f| f[0] * f[0]).sum::<f64>()
            / (output.len() - 100) as f64)
            .sqrt();
        assert!(rms < 0.01, "{rms}");
        let dc = convert(&vec![[0.25, -0.75]; 4000], 44100, 96000, 43);
        assert!(
            dc[100..]
                .iter()
                .all(|f| (f[0] - 0.25).abs() < 1e-10 && (f[1] + 0.75).abs() < 1e-10)
        );
    }

    #[test]
    fn bounded_storage_and_invalid_configuration() {
        assert!(StereoResampler::new(0, 48000, 64).is_err());
        assert!(StereoResampler::new(48000, 48000, 32).is_err());
        let mut src = StereoResampler::new(48000, 48000, 64).unwrap();
        assert_eq!(src.queued_frames(), 0);
        assert_eq!(src.push(&[[0.0; 2]; 128]), 64 - HALF);
        assert_eq!(src.queued_frames(), 64 - HALF);
        assert!(!src.push_frame([0.0; 2]));
        src.set_correction_ppm(f64::INFINITY);
        assert_eq!(src.produce(&mut [[0.0; 2]; 1]), 1);
        assert!(src.free_frames() > 0);
    }
}
