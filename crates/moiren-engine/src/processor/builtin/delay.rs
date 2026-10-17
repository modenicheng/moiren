use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq)]
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
    const MAX_DELAY_MS: f64 = 60_000.0;

    pub fn new(
        channels: usize,
        settings: DelaySettings,
        sample_rate: f64,
    ) -> Result<Self, ProcessorError> {
        if channels == 0
            || !settings.max_delay_ms.is_finite()
            || !(0.0..=Self::MAX_DELAY_MS).contains(&settings.max_delay_ms)
            || !sample_rate.is_finite()
            || sample_rate <= 0.0
        {
            return Err(ProcessorError::InvalidIo);
        }
        let capacity = (settings.max_delay_ms * sample_rate / 1_000.0).ceil() as usize + 1;
        let len = channels
            .checked_mul(capacity)
            .ok_or(ProcessorError::InvalidIo)?;
        Ok(Self {
            channels,
            ring: vec![0.0; len].into_boxed_slice(),
            write: 0,
            capacity,
        })
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

    fn sample_index(&self, channel: usize, delay_frames: usize) -> usize {
        let read = (self.write + self.capacity - (delay_frames % self.capacity)) % self.capacity;
        channel * self.capacity + read
    }

    fn write_index(&self, channel: usize) -> usize {
        channel * self.capacity + self.write
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
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated delay input");
                let mut output = outputs.get_mut(0).expect("validated delay output");
                for frame in 0..input.frames() {
                    let delay_frames = (delay.sample(frame).max(0.0) * ctx.processing_sr / 1_000.0)
                        .round() as usize;
                    let feedback = feedback.sample(frame).clamp(-0.999, 0.999);
                    let mix = mix.sample(frame).clamp(0.0, 1.0);
                    for channel in 0..self.channels {
                        let input_sample = input.channel(channel)[frame].to_f64();
                        let index = self.sample_index(channel, delay_frames);
                        let delayed = if delay_frames == 0 {
                            input_sample
                        } else {
                            self.ring[index]
                        };
                        self.ring[self.write_index(channel)] = input_sample + delayed * feedback;
                        output.channel_mut(channel)[frame] =
                            S::from_f64(input_sample * (1.0 - mix) + delayed * mix);
                    }
                    self.write = (self.write + 1) % self.capacity;
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated delay pair");
                for frame in 0..block.frames() {
                    let delay_frames = (delay.sample(frame).max(0.0) * ctx.processing_sr / 1_000.0)
                        .round() as usize;
                    let feedback = feedback.sample(frame).clamp(-0.999, 0.999);
                    let mix = mix.sample(frame).clamp(0.0, 1.0);
                    for channel in 0..self.channels {
                        let input_sample = block.channel(channel)[frame].to_f64();
                        let index = self.sample_index(channel, delay_frames);
                        let delayed = if delay_frames == 0 {
                            input_sample
                        } else {
                            self.ring[index]
                        };
                        self.ring[self.write_index(channel)] = input_sample + delayed * feedback;
                        block.channel_mut(channel)[frame] =
                            S::from_f64(input_sample * (1.0 - mix) + delayed * mix);
                    }
                    self.write = (self.write + 1) % self.capacity;
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("delay requires an output"),
        }
    }
}
