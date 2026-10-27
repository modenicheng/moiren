use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::MAX_CHANNELS;
use super::dynamics::resolve_float;
use crate::{
    buffer::{FrameIo, PreparedIo, ProcessIo},
    control::{FloatRamp, ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DelaySettings {
    pub delay_ms: f64,
    pub feedback: f64,
    pub mix: f64,
    pub max_delay_ms: f64,
}

impl Default for DelaySettings {
    fn default() -> Self {
        Self {
            delay_ms: 250.0,
            feedback: 0.0,
            mix: 1.0,
            max_delay_ms: 2_000.0,
        }
    }
}

impl DelaySettings {
    /// Single source for the static storage bound: `max_delay_ms` must be
    /// finite and within [`Delay::MAX_DELAY_MS`]. Kernel preparation and every
    /// control-side validator route through here so the three cannot drift.
    pub(crate) fn storage_valid(&self) -> bool {
        self.max_delay_ms.is_finite() && (0.0..=Delay::MAX_DELAY_MS).contains(&self.max_delay_ms)
    }
}

/// A bounded, interpolated-free delay line. The ring is allocated while the
/// plan is prepared; process only performs fixed-size channel/frame work.
#[derive(Debug)]
pub struct Delay {
    channels: usize,
    ring: Box<[f64]>,
    write: usize,
    capacity: usize,
}

impl Delay {
    pub const DELAY: ParameterId = ParameterId(0);
    pub const FEEDBACK: ParameterId = ParameterId(1);
    pub const MIX: ParameterId = ParameterId(2);
    /// Shared bound for `max_delay_ms`; validation routes through
    /// [`DelaySettings::storage_valid`] instead of repeating the literal.
    pub(crate) const MAX_DELAY_MS: f64 = 60_000.0;
    /// Delay history is separate from the compiler's audio slab budget.
    pub const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

    /// Single source for the delay storage contract: channel width, ms range,
    /// frame math that must stay representable, and a ring inside the byte
    /// budget. `new` and the control-side rack precheck both route through
    /// here, so neither can accept storage the other would reject.
    pub(crate) fn validate_settings(
        channels: usize,
        settings: DelaySettings,
        sample_rate: f64,
    ) -> Result<(), ProcessorError> {
        if !(1..=MAX_CHANNELS).contains(&channels)
            || !settings.storage_valid()
            || !sample_rate.is_finite()
            || sample_rate <= 0.0
        {
            return Err(ProcessorError::InvalidIo);
        }
        let frames = (settings.max_delay_ms * sample_rate / 1_000.0).ceil();
        if !frames.is_finite() || frames >= usize::MAX as f64 {
            return Err(ProcessorError::InvalidIo);
        }
        let bytes = (frames as usize)
            .checked_add(1)
            .and_then(|capacity| capacity.checked_mul(channels))
            .and_then(|samples| samples.checked_mul(std::mem::size_of::<f64>()));
        if bytes.is_none_or(|bytes| bytes > Self::MAX_BUFFER_BYTES || bytes > isize::MAX as usize) {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    pub fn new(
        channels: usize,
        settings: DelaySettings,
        sample_rate: f64,
    ) -> Result<Self, ProcessorError> {
        Self::validate_settings(channels, settings, sample_rate)?;
        // All arithmetic is representable per validate_settings.
        let capacity = (settings.max_delay_ms * sample_rate / 1_000.0).ceil() as usize + 1;
        let len = channels * capacity;
        // Preparation can reject an unavailable allocation instead of aborting
        // the process; no allocation or resizing occurs during audio callbacks.
        let mut ring = Vec::new();
        ring.try_reserve_exact(len)
            .map_err(|_| ProcessorError::InvalidIo)?;
        ring.resize(len, 0.0);
        Ok(Self {
            channels,
            ring: ring.into_boxed_slice(),
            write: 0,
            capacity,
        })
    }

    /// Unified preparation entry used by rack expansion and node lowering:
    /// validate storage, build the ring, and emit the kernel's parameter
    /// schema in one step, so a prepared kernel and its table cannot drift
    /// apart. Storage is rejected before the ring is allocated.
    pub fn prepare(
        channels: usize,
        settings: DelaySettings,
        sample_rate: f64,
        id: ProcessorId,
    ) -> Result<(Self, Vec<ParamSpec>), ProcessorError> {
        Self::validate_settings(channels, settings, sample_rate)?;
        let kernel = Self::new(channels, settings, sample_rate)?;
        Ok((kernel, Self::parameters(id, settings).to_vec()))
    }

    pub fn parameters(id: ProcessorId, settings: DelaySettings) -> [ParamSpec; 3] {
        [
            ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: Self::DELAY,
                },
                domain: ParamDomain::Float {
                    min: 0.0,
                    max: settings.max_delay_ms,
                },
                initial: ParamValue::Float(settings.delay_ms),
            },
            ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: Self::FEEDBACK,
                },
                domain: ParamDomain::Float {
                    min: -0.999,
                    max: 0.999,
                },
                initial: ParamValue::Float(settings.feedback),
            },
            ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: Self::MIX,
                },
                domain: ParamDomain::Float { min: 0.0, max: 1.0 },
                initial: ParamValue::Float(settings.mix),
            },
        ]
    }

    /// Effective settings after applying recorded parameter intent by id.
    /// `max_delay_ms` is storage configuration, not a parameter, so intent
    /// never rewrites it; the result feeds both the default ring construction
    /// and [`Delay::parameters`].
    pub(crate) fn resolve_settings(
        settings: DelaySettings,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> DelaySettings {
        DelaySettings {
            delay_ms: resolve_float(&mut intent, Self::DELAY, settings.delay_ms),
            feedback: resolve_float(&mut intent, Self::FEEDBACK, settings.feedback),
            mix: resolve_float(&mut intent, Self::MIX, settings.mix),
            max_delay_ms: settings.max_delay_ms,
        }
    }

    fn sample_index(&self, channel: usize, delay_frames: usize) -> usize {
        let read = (self.write + self.capacity - (delay_frames % self.capacity)) % self.capacity;
        channel * self.capacity + read
    }

    fn write_index(&self, channel: usize) -> usize {
        channel * self.capacity + self.write
    }
    /// One kernel for both transports: each channel sample is read before the
    /// ring is updated and the frame position is written.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(
        &mut self,
        io: &mut Io,
        delay: FloatRamp,
        feedback: FloatRamp,
        mix: FloatRamp,
        sample_rate: f64,
    ) {
        for frame in 0..io.frames() {
            let delay_frames =
                (delay.sample(frame).max(0.0) * sample_rate / 1_000.0).round() as usize;
            let feedback = feedback.sample(frame).clamp(-0.999, 0.999);
            let mix = mix.sample(frame).clamp(0.0, 1.0);
            for channel in 0..self.channels {
                let input_sample = io.read(channel, frame);
                let index = self.sample_index(channel, delay_frames);
                let delayed = if delay_frames == 0 {
                    input_sample
                } else {
                    self.ring[index]
                };
                self.ring[self.write_index(channel)] = input_sample + delayed * feedback;
                io.write(
                    channel,
                    frame,
                    S::from_f64(input_sample * (1.0 - mix) + delayed * mix),
                );
            }
            self.write = (self.write + 1) % self.capacity;
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Delay {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        let channels = io.input_channels(0).ok_or(ProcessorError::InvalidIo)?;
        if io.input_count() != 1
            || io.output_count() != 1
            || io.output_channels(0) != Some(channels)
            || channels != self.channels
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        for parameter in [Self::DELAY, Self::FEEDBACK, Self::MIX] {
            params
                .float(parameter)
                .ok_or(ProcessorError::MissingParameter)?;
        }
        Ok(())
    }

    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let delay = params
            .float(Self::DELAY)
            .expect("validated delay parameter");
        let feedback = params
            .float(Self::FEEDBACK)
            .expect("validated feedback parameter");
        let mix = params.float(Self::MIX).expect("validated mix parameter");
        linked_io!(io, self, "delay", delay, feedback, mix, ctx.processing_sr)
    }
}
