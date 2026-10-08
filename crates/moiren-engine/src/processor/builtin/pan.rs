use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

/// Stereo balance. Center preserves both channels; positive positions attenuate
/// left, negative positions attenuate right, without crossfeeding or boosting.
/// This is not a mono-to-stereo equal-power panner.
pub struct Pan;

impl Pan {
    pub const POSITION: ParameterId = ParameterId(0);

    pub fn parameter(id: ProcessorId, initial: f64) -> ParamSpec {
        ParamSpec {
            key: ParameterKey {
                processor: id,
                parameter: Self::POSITION,
            },
            domain: ParamDomain::Float {
                min: -1.0,
                max: 1.0,
            },
            initial: ParamValue::Float(initial),
        }
    }

    fn coefficient(channel: usize, position: f64) -> f64 {
        if channel == 0 {
            1.0 - position.max(0.0)
        } else {
            1.0 + position.min(0.0)
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Pan {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(2)
            || io.output_channels(0) != Some(2)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        params
            .float(Self::POSITION)
            .ok_or(ProcessorError::MissingParameter)?;
        match params.domain(Self::POSITION) {
            Some(ParamDomain::Float { min, max }) if min >= -1.0 && max <= 1.0 => Ok(()),
            _ => Err(ProcessorError::InvalidParameterDomain),
        }
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
    ) {
        let position = params
            .float(Self::POSITION)
            .expect("validated pan parameter");
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated stereo input");
                let mut output = outputs.get_mut(0).expect("validated stereo output");
                for (channel, (src, dst)) in input.channels().zip(output.channels_mut()).enumerate()
                {
                    for (i, (src, dst)) in src.iter().zip(dst.iter_mut()).enumerate() {
                        *dst = S::from_f64(
                            src.to_f64() * Self::coefficient(channel, position.sample(i)),
                        );
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated stereo pair");
                for (channel, samples) in block.channels_mut().enumerate() {
                    for (i, sample) in samples.iter_mut().enumerate() {
                        *sample = S::from_f64(
                            sample.to_f64() * Self::coefficient(channel, position.sample(i)),
                        );
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("pan requires an output"),
        }
    }
}
