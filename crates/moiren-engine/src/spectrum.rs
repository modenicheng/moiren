//! Demand-gated, per-channel exact source-frame windows with an off-thread
//! transform. The realtime half ([`SignalSpectrumAnalyzer`]) only keeps a
//! history ring and publishes each hop's exact window; [`SpectrumAnalysis`]
//! runs the FFT wherever that window is read, so the transform never charges
//! the audio callback.

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

const fn valid_fft_size(fft_size: usize) -> bool {
    fft_size >= MIN_FFT_SIZE && fft_size <= MAX_FFT_SIZE && fft_size.is_power_of_two()
}

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
        valid_fft_size(self.fft_size) && self.hop_frames != 0 && self.hop_frames <= self.fft_size
    }

    /// Number of nonnegative-frequency bins in one channel's analyzed spectrum.
    pub const fn bins_per_channel(self) -> usize {
        self.fft_size / 2 + 1
    }
}

impl Default for SpectrumSettings {
    /// 8192 points at 48 kHz resolve 5.9 Hz per bin, so the 1/12-octave
    /// display grid sits above the transform's resolution from ~100 Hz up;
    /// a 1024-frame hop keeps ~47 windows a second. Affordable because the
    /// transform runs off the audio thread.
    fn default() -> Self {
        Self {
            fft_size: 8192,
            hop_frames: 1024,
        }
    }
}

/// Exact source timeline represented by one channel-major window block.
///
/// `frame_end` is exclusive. The realtime tap publishes `fft_size`
/// consecutive `f32` source frames per channel, oldest first;
/// [`SpectrumAnalysis`] turns such a block into the same header over
/// `fft_size / 2 + 1` one-sided magnitudes per channel at frequencies
/// `bin * sample_rate / fft_size`: a periodic Hann window divided by its
/// coherent sum, interior bins doubled to report one-sided peak amplitude
/// while DC and Nyquist are not.
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
    #[error("publisher capacity or block length does not match the channel-major shape")]
    CapacityMismatch,
}

/// Realtime half of the spectrum analyzer: keeps the last `fft_size` source
/// frames per channel and publishes them as one exact-window block every
/// `hop_frames` frames. All history is prepared off RT; observing only
/// converts samples and copies memory.
pub struct SignalSpectrumAnalyzer {
    channels: usize,
    settings: SpectrumSettings,
    ring: Box<[f32]>,
    ring_write: usize,
    frames_until_window: usize,
    continuity: WindowContinuity,
    publisher: PublisherHandle<LatestBlock<SpectrumWindow, f32>>,
    bindings: Box<[ErasedBinding]>,
}

impl SignalSpectrumAnalyzer {
    /// Prepare a publisher with capacity `channels * settings.fft_size`.
    ///
    /// Channels are captured independently. Construction, binding retention
    /// and eventual destruction belong on the control thread.
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
        let history_len = channels
            .checked_mul(settings.fft_size)
            .filter(|&len| len <= isize::MAX as usize / std::mem::size_of::<f32>())
            .ok_or(SpectrumError::InvalidChannels)?;
        if publisher.capacity() != history_len {
            return Err(SpectrumError::CapacityMismatch);
        }

        Ok(Self {
            channels,
            settings,
            ring: vec![0.0; history_len].into_boxed_slice(),
            ring_write: 0,
            frames_until_window: settings.fft_size,
            continuity: WindowContinuity::new(),
            publisher: publisher.handle(),
            bindings: vec![publisher.erase()].into_boxed_slice(),
        })
    }

    fn reset_history(&mut self) {
        // A full window overwrites every old sample before any publication;
        // resetting cursors is sufficient even when overlapping history exists.
        self.ring_write = 0;
        self.frames_until_window = self.settings.fft_size;
    }

    fn publish_window(&mut self, signals: &mut ProcessSignals<'_>, frame_end: u64) {
        let len = self.ring.len();
        let Ok(mut write) = signals.try_write(&self.publisher, len) else {
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
        let fft_size = self.settings.fft_size;
        let ring_write = self.ring_write;
        for (history, window) in self
            .ring
            .chunks_exact(fft_size)
            .zip(write.elements_mut().chunks_exact_mut(fft_size))
        {
            // Unroll the ring so the window reads oldest frame first.
            let (newest, oldest) = history.split_at(ring_write);
            window[..oldest.len()].copy_from_slice(oldest);
            window[oldest.len()..].copy_from_slice(newest);
        }
        let _ = write.commit(len);
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
        let fft_size = self.settings.fft_size;
        for frame in 0..ctx.frames {
            for channel in 0..self.channels {
                let index = channel * fft_size + self.ring_write;
                self.ring[index] = input.channel(channel)[frame].to_f64() as f32;
            }
            self.ring_write = (self.ring_write + 1) % fft_size;
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

/// Off-thread half of the spectrum analyzer: transforms one exact-window
/// block into one-sided magnitudes with a periodic Hann window, a radix-2
/// FFT and coherent-gain scaling. Owns its workspace, so construction
/// allocates and `analyze` never does.
pub struct SpectrumAnalysis {
    channels: usize,
    fft_size: usize,
    window: Box<[f64]>,
    coherent_scale: f64,
    fft: Radix2Fft,
}

impl SpectrumAnalysis {
    pub fn new(channels: usize, fft_size: usize) -> Result<Self, SpectrumError> {
        if channels == 0 {
            return Err(SpectrumError::InvalidChannels);
        }
        if !valid_fft_size(fft_size) {
            return Err(SpectrumError::InvalidSettings);
        }
        channels
            .checked_mul(fft_size)
            .ok_or(SpectrumError::InvalidChannels)?;

        let window: Box<[f64]> = (0..fft_size)
            .map(|frame| 0.5 - 0.5 * (std::f64::consts::TAU * frame as f64 / fft_size as f64).cos())
            .collect();
        let coherent_sum: f64 = window.iter().sum();
        let coherent_scale = 2.0 / coherent_sum;

        Ok(Self {
            channels,
            fft_size,
            window,
            coherent_scale,
            fft: Radix2Fft::new(fft_size),
        })
    }

    /// Frames one analysis consumes: `channels * fft_size`, channel-major.
    pub fn window_len(&self) -> usize {
        self.channels * self.fft_size
    }

    /// Magnitudes one analysis produces: `channels * (fft_size / 2 + 1)`,
    /// channel-major, as published with a [`SpectrumWindow`] header.
    pub fn magnitudes_len(&self) -> usize {
        self.channels * (self.fft_size / 2 + 1)
    }

    /// Transform the realtime tap's `frames` block into `magnitudes`; both
    /// must be exactly [`Self::window_len`] and [`Self::magnitudes_len`]
    /// long.
    pub fn analyze(&mut self, frames: &[f32], magnitudes: &mut [f32]) -> Result<(), SpectrumError> {
        if frames.len() != self.window_len() || magnitudes.len() != self.magnitudes_len() {
            return Err(SpectrumError::CapacityMismatch);
        }
        let bins_per_channel = self.fft_size / 2 + 1;
        for (history, output) in frames
            .chunks_exact(self.fft_size)
            .zip(magnitudes.chunks_exact_mut(bins_per_channel))
        {
            self.fft.transform(history, &self.window);
            for (bin, magnitude) in output.iter_mut().enumerate() {
                let scale = if bin == 0 || bin == self.fft_size / 2 {
                    self.coherent_scale * 0.5
                } else {
                    self.coherent_scale
                };
                *magnitude = (self.fft.re[bin].hypot(self.fft.im[bin]) * scale) as f32;
            }
        }
        Ok(())
    }
}
