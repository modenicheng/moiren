//! Streaming windowed-sinc SRC with fixed storage and precomputed phase kernels.
//! No allocation, trigonometry or reclamation occurs while moving samples.
use std::f64::consts::PI;

use super::{codec::finite, native::StereoFrame};
use thiserror::Error;

const TAPS: usize = 32;
const HALF: usize = TAPS / 2;
const PHASES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("SRC rates must be 8..384 kHz, and capacity must be 64..1048576 frames")]
pub struct ResampleError;

pub struct StereoResampler {
    frames: Box<[StereoFrame]>,
    kernels: Box<[[f64; TAPS]]>,
    head: usize,
    len: usize,
    position: f64,
    nominal_step: f64,
    step: f64,
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
        })
    }

    pub fn capacity(&self) -> usize {
        self.frames.len()
    }
    pub fn queued_frames(&self) -> usize {
        self.len.saturating_sub(self.position.floor() as usize)
    }
    pub fn free_frames(&self) -> usize {
        self.frames.len() - self.len
    }

    /// Positive correction consumes source faster to reduce queue occupancy.
    pub fn set_correction_ppm(&mut self, ppm: f64) {
        self.step = self.nominal_step * (1.0 + finite(ppm).clamp(-2000.0, 2000.0) * 1e-6);
    }

    pub fn push_frame(&mut self, frame: StereoFrame) -> bool {
        if self.len == self.frames.len() {
            return false;
        }
        let index = (self.head + self.len) % self.frames.len();
        self.frames[index] = [finite(frame[0]), finite(frame[1])];
        self.len += 1;
        true
    }

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
        Some(output)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(src.push(&[[0.0; 2]; 128]), 64 - HALF);
        assert!(!src.push_frame([0.0; 2]));
        src.set_correction_ppm(f64::INFINITY);
        assert!(src.next_frame().is_some());
        assert!(src.free_frames() > 0);
    }
}
