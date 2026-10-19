//! Demand-gated, per-channel FFT magnitude over exact source-frame windows.

mod fft;

#[cfg(test)]
mod tests;

use crate::{
    buffer::{PreparedIo, ReadPorts},
    processor::{
        ProcessContext, ProcessorError, RtObserver, TimelineHeader, WindowContinuity,
        validate_single_input,
    },
    sample::ProcessingSample,
    signal::ProcessSignals,
};
use fft::Radix2Fft;
use moiren_signal::{ErasedBinding, LatestBlock, PreparedPublisher, PublisherHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MIN_FFT_SIZE: usize = 16;
const MAX_FFT_SIZE: usize = 8192;

/// Bounded FFT and overlap configuration for a realtime spectrum observer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpectrumSettings {
    pub fft_size: usize,
    pub hop_frames: usize,
}

impl SpectrumSettings {
    /// Whether this FFT size and hop can be prepared within the supported bounds.
    pub const fn valid(self) -> bool {
        self.fft_size >= MIN_FFT_SIZE
            && self.fft_size <= MAX_FFT_SIZE
            && self.fft_size.is_power_of_two()
            && self.hop_frames != 0
            && self.hop_frames <= self.fft_size
    }

    /// Number of nonnegative-frequency bins in one channel's real spectrum.
    pub const fn bins_per_channel(self) -> usize {
        self.fft_size / 2 + 1
    }
}

impl Default for SpectrumSettings {
    fn default() -> Self {
        Self {
            fft_size: 1024,
            hop_frames: 512,
        }
    }
}

/// Exact source timeline represented by one channel-major spectrum block.
///
/// `frame_end` is exclusive. Each channel occupies `fft_size / 2 + 1` consecutive
/// `f32` bins at frequencies `bin * sample_rate / fft_size`. Amplitudes use a
/// periodic Hann window, divided by its coherent sum; interior bins are doubled
/// to report one-sided peak amplitude while DC and Nyquist are not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumWindow {
    pub timeline_epoch: u64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub sample_rate: f64,
    pub fft_size: usize,
    pub channels: usize,
}

impl From<TimelineHeader> for SpectrumWindow {
    fn from(header: TimelineHeader) -> Self {
        // Kernel-specific fields are set by the constructing write; the
        // conversion only fills the shared timeline prefix.
        let TimelineHeader {
            timeline_epoch,
            frame_start,
            frame_end,
            sample_rate,
        } = header;
        Self {
            timeline_epoch,
            frame_start,
            frame_end,
            sample_rate,
            fft_size: 0,
            channels: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SpectrumError {
    #[error("channel count must be nonzero and representable")]
    InvalidChannels,
    #[error("FFT size or hop is outside the supported bounds")]
    InvalidSettings,
    #[error("topic capacity must equal channels times bins per channel")]
    CapacityMismatch,
}

/// Read-only per-channel spectrum; all histories and FFT workspaces are prepared off RT.
pub struct SignalSpectrumAnalyzer {
    channels: usize,
    settings: SpectrumSettings,
    bins_per_channel: usize,
    ring: Box<[f64]>,
    ring_write: usize,
    frames_until_window: usize,
    continuity: WindowContinuity,
    window: Box<[f64]>,
    coherent_scale: f64,
    fft: Radix2Fft,
    publisher: PublisherHandle<LatestBlock<SpectrumWindow, f32>>,
    bindings: Box<[ErasedBinding]>,
}

impl SignalSpectrumAnalyzer {
    /// Prepare a publisher with capacity `channels * settings.bins_per_channel()`.
    ///
    /// Channels are measured independently. Construction, binding retention and
    /// eventual destruction belong on the control thread.
    pub fn new(
        channels: usize,
        settings: SpectrumSettings,
        publisher: PreparedPublisher<LatestBlock<SpectrumWindow, f32>>,
    ) -> Result<Self, SpectrumError> {
        if channels == 0 {
            return Err(SpectrumError::InvalidChannels);
        }
        if !settings.valid() {
            return Err(SpectrumError::InvalidSettings);
        }
        let bins_per_channel = settings.bins_per_channel();
        let Some(capacity) = channels.checked_mul(bins_per_channel) else {
            return Err(SpectrumError::InvalidChannels);
        };
        let history_len = channels
            .checked_mul(settings.fft_size)
            .filter(|&len| len <= isize::MAX as usize / std::mem::size_of::<f64>())
            .ok_or(SpectrumError::InvalidChannels)?;
        if publisher.capacity() != capacity {
            return Err(SpectrumError::CapacityMismatch);
        }

        let window: Box<[f64]> = (0..settings.fft_size)
            .map(|frame| {
                0.5 - 0.5 * (std::f64::consts::TAU * frame as f64 / settings.fft_size as f64).cos()
            })
            .collect();
        let coherent_sum: f64 = window.iter().sum();
        let coherent_scale = 2.0 / coherent_sum;

        Ok(Self {
            channels,
            settings,
            bins_per_channel,
            ring: vec![0.0; history_len].into_boxed_slice(),
            ring_write: 0,
            frames_until_window: settings.fft_size,
            continuity: WindowContinuity::new(),
            window,
            coherent_scale,
            fft: Radix2Fft::new(settings.fft_size),
            publisher: publisher.handle(),
            bindings: vec![publisher.erase()].into_boxed_slice(),
        })
    }

    fn reset_history(&mut self) {
        // A full FFT window overwrites every old sample before any transform;
        // resetting cursors is sufficient even when overlapping history exists.
        self.ring_write = 0;
        self.frames_until_window = self.settings.fft_size;
    }

    fn publish_window(&mut self, signals: &mut ProcessSignals<'_>, frame_end: u64) {
        let Ok(mut write) =
            signals.try_write(&self.publisher, self.channels * self.bins_per_channel)
        else {
            return;
        };

        let header = self
            .continuity
            .window(frame_end - self.settings.fft_size as u64, frame_end)
            .into();
        *write.header_mut() = SpectrumWindow {
            fft_size: self.settings.fft_size,
            channels: self.channels,
            ..header
        };
        let elements = write.elements_mut();
        for channel in 0..self.channels {
            let ring_start = channel * self.settings.fft_size;
            self.fft.transform_ring(
                &self.ring[ring_start..ring_start + self.settings.fft_size],
                self.ring_write,
                &self.window,
            );
            let output_start = channel * self.bins_per_channel;
            let output = &mut elements[output_start..output_start + self.bins_per_channel];
            for (bin, amplitude) in output.iter_mut().enumerate() {
                let scale = if bin == 0 || bin == self.settings.fft_size / 2 {
                    self.coherent_scale * 0.5
                } else {
                    self.coherent_scale
                };
                *amplitude = (self.fft.re[bin].hypot(self.fft.im[bin]) * scale) as f32;
            }
        }
        let _ = write.commit(self.channels * self.bins_per_channel);
    }
}

impl<S: ProcessingSample> RtObserver<S> for SignalSpectrumAnalyzer {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_single_input(self.channels, io)
    }

    fn signal_bindings(&self) -> &[ErasedBinding] {
        &self.bindings
    }

    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        mut signals: ProcessSignals<'_>,
    ) {
        let Some(restart) = self
            .continuity
            .begin_window(&mut signals, &self.publisher, ctx)
        else {
            return;
        };
        if restart {
            self.reset_history();
        }

        let input = inputs.get(0).expect("validated spectrum analyzer input");
        for frame in 0..ctx.frames {
            for channel in 0..self.channels {
                let index = channel * self.settings.fft_size + self.ring_write;
                self.ring[index] = input.channel(channel)[frame].to_f64();
            }
            self.ring_write = (self.ring_write + 1) % self.settings.fft_size;
            self.frames_until_window -= 1;

            if self.frames_until_window == 0 {
                let frame_end = ctx.timeline_start + frame as u64 + 1;
                self.publish_window(&mut signals, frame_end);
                self.frames_until_window = self.settings.hop_frames;
            }
        }
        self.continuity.finish_block(ctx);
    }
}
