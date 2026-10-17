use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::MAX_CHANNELS;
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimiterSettings {
    pub threshold_db: f64,
    pub release_ms: f64,
    pub input_gain_db: f64,
}

impl Default for LimiterSettings {
    fn default() -> Self {
        Self {
            threshold_db: -1.0,
            release_ms: 50.0,
            input_gain_db: 0.0,
        }
    }
}

/// Zero-latency linked peak limiter. Attack is immediate; release is stateful.
#[derive(Debug)]
pub struct Limiter {
    gain: f64,
}

impl Default for Limiter {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

impl Limiter {
    pub const THRESHOLD: ParameterId = ParameterId(0);
    pub const RELEASE: ParameterId = ParameterId(1);
    pub const INPUT_GAIN: ParameterId = ParameterId(2);

    const DOMAINS: [(f64, f64); 3] = [(-60.0, 0.0), (0.0, 5_000.0), (-60.0, 60.0)];

    pub fn new() -> Self {
        Self::default()
    }

    pub fn parameters(id: ProcessorId, settings: LimiterSettings) -> [ParamSpec; 3] {
        let values = [settings.threshold_db, settings.release_ms, settings.input_gain_db];
        std::array::from_fn(|index| {
            let (min, max) = Self::DOMAINS[index];
            ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: ParameterId(index as u32),
                },
                domain: ParamDomain::Float { min, max },
                initial: ParamValue::Float(values[index]),
            }
        })
    }

    fn gain_from_db(db: f64) -> f64 {
        10.0_f64.powf(db / 20.0)
    }

    fn sample_rate(sample_rate: f64) -> f64 {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        }
    }

    fn release(current: f64, target: f64, release_ms: f64, sample_rate: f64) -> f64 {
        if release_ms == 0.0 {
            target
        } else {
            let samples = (release_ms * Self::sample_rate(sample_rate) / 1_000.0).max(1.0);
            current + (target - current) * (1.0 - (-1.0 / samples).exp())
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Limiter {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        let channels = io.input_channels(0).ok_or(ProcessorError::InvalidIo)?;
        if io.input_count() != 1
            || io.output_count() != 1
            || io.output_channels(0) != Some(channels)
            || channels == 0
            || channels > MAX_CHANNELS
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        for index in 0..3 {
            params
                .float(ParameterId(index))
                .ok_or(ProcessorError::MissingParameter)?;
            let (supported_min, supported_max) = Self::DOMAINS[index as usize];
            match params.domain(ParameterId(index)) {
                Some(ParamDomain::Float { min, max })
                    if min.is_finite()
                        && max.is_finite()
                        && min <= max
                        && min >= supported_min
                        && max <= supported_max => {}
                _ => return Err(ProcessorError::InvalidParameterDomain),
            }
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
        let threshold = params
            .float(Self::THRESHOLD)
            .expect("validated limiter threshold");
        let release = params
            .float(Self::RELEASE)
            .expect("validated limiter release");
        let input_gain = params
            .float(Self::INPUT_GAIN)
            .expect("validated limiter input gain");
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated limiter input");
                let mut output = outputs.get_mut(0).expect("validated limiter output");
                for frame in 0..input.frames() {
                    let input_gain = Self::gain_from_db(input_gain.sample(frame));
                    let threshold = Self::gain_from_db(threshold.sample(frame));
                    let peak = input.channels().fold(0.0_f64, |peak, channel| {
                        peak.max(channel[frame].to_f64().abs() * input_gain)
                    });
                    let target = if peak > threshold {
                        threshold / peak
                    } else {
                        1.0
                    };
                    self.gain = if target < self.gain {
                        target
                    } else {
                        Self::release(self.gain, target, release.sample(frame), ctx.processing_sr)
                    };
                    for channel in 0..input.channel_count() {
                        output.channel_mut(channel)[frame] = S::from_f64(
                            input.channel(channel)[frame].to_f64() * input_gain * self.gain,
                        );
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated limiter pair");
                for frame in 0..block.frames() {
                    let input_gain = Self::gain_from_db(input_gain.sample(frame));
                    let threshold = Self::gain_from_db(threshold.sample(frame));
                    let peak = (0..block.channel_count()).fold(0.0_f64, |peak, channel| {
                        peak.max(block.channel(channel)[frame].to_f64().abs() * input_gain)
                    });
                    let target = if peak > threshold {
                        threshold / peak
                    } else {
                        1.0
                    };
                    self.gain = if target < self.gain {
                        target
                    } else {
                        Self::release(self.gain, target, release.sample(frame), ctx.processing_sr)
                    };
                    for channel in block.channels_mut() {
                        channel[frame] =
                            S::from_f64(channel[frame].to_f64() * input_gain * self.gain);
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("limiter requires an output"),
        }
    }
}
