use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::resolve_float;
use crate::{
    buffer::{FrameIo, InPlaceFrames, PreparedIo, ProcessIo, SeparateFrames},
    control::{FloatRamp, ParamDomain, ParamSpec, ProcessParameters},
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

    /// One kernel for both transports; the ramp is sampled once per frame.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(io: &mut Io, level: FloatRamp) {
        for frame in 0..io.frames() {
            let gain = level.sample(frame);
            for channel in 0..io.channel_count() {
                io.write(channel, frame, S::from_f64(io.read(channel, frame) * gain));
            }
        }
    }

    /// Effective level after applying recorded parameter intent by id; the
    /// result feeds both the default kernel construction and
    /// [`Gain::parameter`].
    pub(crate) fn resolve_level(
        initial: f64,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> f64 {
        resolve_float(&mut intent, Self::LEVEL, initial)
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
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let gain = params.float(Self::LEVEL).expect("validated gain parameter");
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => Self::process_linked(
                &mut SeparateFrames::new(
                    inputs.get(0).expect("validated main input"),
                    outputs.get_mut(0).expect("validated main output"),
                ),
                gain,
            ),
            ProcessIo::InPlace { mut pairs, .. } => Self::process_linked(
                &mut InPlaceFrames::new(pairs.get_mut(0, 0).expect("validated main pair")),
                gain,
            ),
            ProcessIo::ReadOnly { .. } => unreachable!("gain requires an output"),
        }
    }
}
