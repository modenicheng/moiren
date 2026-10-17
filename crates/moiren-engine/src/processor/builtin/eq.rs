use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::MAX_CHANNELS;
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqSettings {
    pub frequency_hz: f64,
    pub gain_db: f64,
    pub q: f64,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            frequency_hz: 1_000.0,
            gain_db: 0.0,
            q: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct EqState {
    input_1: f64,
    input_2: f64,
    output_1: f64,
    output_2: f64,
}

/// One linked-free parametric peaking band. State is kept per channel so the
/// filter remains safe for in-place processing without a temporary buffer.
#[derive(Debug)]
pub struct Eq {
    states: [EqState; MAX_CHANNELS],
}

impl Eq {
    pub const FREQUENCY: ParameterId = ParameterId(0);
    pub const GAIN: ParameterId = ParameterId(1);
    pub const Q: ParameterId = ParameterId(2);

    const DOMAINS: [(f64, f64); 3] = [(20.0, 20_000.0), (-24.0, 24.0), (0.1, 20.0)];

    pub fn new() -> Self {
        Self::default()
    }

    pub fn parameters(id: ProcessorId, settings: EqSettings) -> [ParamSpec; 3] {
        let values = [settings.frequency_hz, settings.gain_db, settings.q];
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

    fn coefficients(frequency_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> [f64; 5] {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let frequency_hz = frequency_hz.clamp(0.0, (sample_rate * 0.49).max(1.0));
        let q = q.max(0.1);
        let omega = 2.0 * std::f64::consts::PI * frequency_hz / sample_rate;
        let sine = omega.sin();
        let alpha = sine / (2.0 * q);
        let cosine = omega.cos();
        let amplitude = 10.0_f64.powf(gain_db / 40.0);
        let a0 = 1.0 + alpha / amplitude;
        [
            (1.0 + alpha * amplitude) / a0,
            (-2.0 * cosine) / a0,
            (1.0 - alpha * amplitude) / a0,
            (-2.0 * cosine) / a0,
            (1.0 - alpha / amplitude) / a0,
        ]
    }

    fn process_sample<S: ProcessingSample>(
        state: &mut EqState,
        sample: S,
        coefficients: [f64; 5],
    ) -> S {
        let input = sample.to_f64();
        let output = coefficients[0] * input
            + coefficients[1] * state.input_1
            + coefficients[2] * state.input_2
            - coefficients[3] * state.output_1
            - coefficients[4] * state.output_2;
        state.input_2 = state.input_1;
        state.input_1 = input;
        state.output_2 = state.output_1;
        state.output_1 = output;
        S::from_f64(output)
    }
}

impl Default for Eq {
    fn default() -> Self {
        Self {
            states: std::array::from_fn(|_| EqState::default()),
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Eq {
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
        let frequency = params
            .float(Self::FREQUENCY)
            .expect("validated EQ frequency parameter");
        let gain = params
            .float(Self::GAIN)
            .expect("validated EQ gain parameter");
        let q = params.float(Self::Q).expect("validated EQ Q parameter");
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated EQ input");
                let mut output = outputs.get_mut(0).expect("validated EQ output");
                for channel in 0..input.channel_count() {
                    let source = input.channel(channel);
                    let destination = output.channel_mut(channel);
                    for frame in 0..source.len() {
                        let coefficients = Self::coefficients(
                            frequency.sample(frame),
                            gain.sample(frame),
                            q.sample(frame),
                            ctx.processing_sr,
                        );
                        destination[frame] = Self::process_sample(
                            &mut self.states[channel],
                            source[frame],
                            coefficients,
                        );
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated EQ pair");
                for channel in 0..block.channel_count() {
                    let samples = block.channel_mut(channel);
                    for (frame, sample) in samples.iter_mut().enumerate() {
                        let coefficients = Self::coefficients(
                            frequency.sample(frame),
                            gain.sample(frame),
                            q.sample(frame),
                            ctx.processing_sr,
                        );
                        *sample =
                            Self::process_sample(&mut self.states[channel], *sample, coefficients);
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("EQ requires an output"),
        }
    }
}
