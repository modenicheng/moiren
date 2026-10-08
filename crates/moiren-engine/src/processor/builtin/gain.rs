use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

pub struct Gain;
impl Gain {
    pub const LEVEL: ParameterId = ParameterId(0);
    pub fn parameter(id: ProcessorId, initial: f64) -> ParamSpec {
        ParamSpec {
            key: ParameterKey {
                processor: id,
                parameter: Self::LEVEL,
            },
            domain: ParamDomain::Float {
                min: 0.0,
                max: 16.0,
            },
            initial: ParamValue::Float(initial),
        }
    }
}
impl<S: ProcessingSample> RtProcessor<S> for Gain {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0).is_none()
            || io.input_channels(0) != io.output_channels(0)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }
    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        params
            .float(Self::LEVEL)
            .map(|_| ())
            .ok_or(ProcessorError::MissingParameter)
    }
    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
    ) {
        let gain = params.float(Self::LEVEL).expect("validated gain parameter");
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated main input");
                let mut output = outputs.get_mut(0).expect("validated main output");
                for (src, dst) in input.channels().zip(output.channels_mut()) {
                    for (i, (src, dst)) in src.iter().zip(dst.iter_mut()).enumerate() {
                        *dst = S::from_f64(src.to_f64() * gain.sample(i));
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated main pair");
                for channel in block.channels_mut() {
                    for (i, sample) in channel.iter_mut().enumerate() {
                        *sample = S::from_f64(sample.to_f64() * gain.sample(i));
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("gain requires an output"),
        }
    }
}
