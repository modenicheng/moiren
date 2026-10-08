//! Route lowering kernel, private to the compiler. No separate editable node.
use super::SendParameterKeys;
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    processor::{ProcessContext, ProcessorError, RtProcessor},
    sample::ProcessingSample,
};
use moiren_core::{
    graph::SendParams,
    protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId},
};

pub(super) struct Send {
    pub channels: usize,
}
impl Send {
    const GAIN: ParameterId = ParameterId(0);
    const PAN: ParameterId = ParameterId(1);
    const MUTE: ParameterId = ParameterId(2);

    pub fn parameters(
        processor: ProcessorId,
        channels: usize,
        initial: SendParams,
    ) -> ([ParamSpec; 3], SendParameterKeys) {
        let gain = ParameterKey {
            processor,
            parameter: Self::GAIN,
        };
        let pan = ParameterKey {
            processor,
            parameter: Self::PAN,
        };
        let mute = ParameterKey {
            processor,
            parameter: Self::MUTE,
        };
        (
            [
                ParamSpec {
                    key: gain,
                    domain: ParamDomain::Float {
                        min: 0.0,
                        max: f64::MAX,
                    },
                    initial: ParamValue::Float(initial.gain),
                },
                ParamSpec {
                    key: pan,
                    domain: ParamDomain::Float {
                        min: if channels == 2 { -1.0 } else { 0.0 },
                        max: if channels == 2 { 1.0 } else { 0.0 },
                    },
                    initial: ParamValue::Float(initial.pan),
                },
                ParamSpec {
                    key: mute,
                    domain: ParamDomain::Bool,
                    initial: ParamValue::Bool(initial.mute),
                },
            ],
            SendParameterKeys { gain, pan, mute },
        )
    }
}
impl<S: ProcessingSample> RtProcessor<S> for Send {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.channels)
            || io.output_channels(0) != Some(self.channels)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }
    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        if params.float(Self::GAIN).is_none()
            || params.float(Self::PAN).is_none()
            || params.discrete(Self::MUTE).is_none()
        {
            return Err(ProcessorError::MissingParameter);
        }
        Ok(())
    }
    fn process(&mut self, _: &ProcessContext, io: ProcessIo<'_, S>, params: ProcessParameters<'_>) {
        if let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        {
            let input = inputs.get(0).expect("validated send input");
            let mut output = outputs.get_mut(0).expect("validated send output");
            if params.discrete(Self::MUTE) == Some(ParamValue::Bool(true)) {
                for samples in output.channels_mut() {
                    samples.fill(S::ZERO);
                }
                return;
            }
            let gain = params.float(Self::GAIN).expect("validated send gain");
            let pan = params.float(Self::PAN).expect("validated send pan");
            for (channel, (src, dst)) in input.channels().zip(output.channels_mut()).enumerate() {
                for (i, (src, dst)) in src.iter().zip(dst.iter_mut()).enumerate() {
                    let position = pan.sample(i);
                    let balance = if self.channels != 2 {
                        1.0
                    } else if channel == 0 {
                        1.0 - position.max(0.0)
                    } else {
                        1.0 + position.min(0.0)
                    };
                    // Balance is in [0, 1]. Form the finite coefficient before
                    // scaling audio, so a fully attenuated channel cannot turn
                    // an avoidable gain overflow into NaN.
                    let coefficient = gain.sample(i) * balance;
                    *dst = S::from_f64(src.to_f64() * coefficient);
                }
            }
        }
    }
}
