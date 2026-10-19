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
    pub const MONO: ParameterId = ParameterId(1);
    pub const SWAP: ParameterId = ParameterId(2);

    pub fn parameters(id: ProcessorId, position: f64, mono: bool, swap: bool) -> [ParamSpec; 3] {
        [
            Self::parameter(id, position),
            Self::boolean_parameter(id, Self::MONO, mono),
            Self::boolean_parameter(id, Self::SWAP, swap),
        ]
    }
    fn boolean_parameter(id: ProcessorId, parameter: ParameterId, initial: bool) -> ParamSpec {
        ParamSpec {
            key: ParameterKey {
                processor: id,
                parameter,
            },
            domain: ParamDomain::Bool,
            initial: ParamValue::Bool(initial),
        }
    }

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

    /// Per-channel balance factor; also the compiler's send kernel, so edge
    /// pan and the Pan node can never drift apart.
    pub(crate) fn coefficient(channel: usize, position: f64) -> f64 {
        if channel == 0 {
            1.0 - position.max(0.0)
        } else {
            1.0 + position.min(0.0)
        }
    }
    /// Transform the balanced stereo result. Averaging last makes Mono a true
    /// dual-mono monitor signal, even at noncentral balance, without boosting.
    fn stereo(left: f64, right: f64, position: f64, mono: bool, swap: bool) -> (f64, f64) {
        let mut left = left * Self::coefficient(0, position);
        let mut right = right * Self::coefficient(1, position);
        if swap {
            std::mem::swap(&mut left, &mut right);
        }
        if mono {
            let value = left * 0.5 + right * 0.5;
            (value, value)
        } else {
            (left, right)
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
        if !matches!(params.domain(Self::POSITION), Some(ParamDomain::Float { min, max }) if min >= -1.0 && max <= 1.0)
        {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        // POSITION-only processors prepared through the older public helper
        // remain valid. When present, switches must use the exact Bool schema.
        for parameter in [Self::MONO, Self::SWAP] {
            if let Some(domain) = params.domain(parameter)
                && (domain != ParamDomain::Bool
                    || !matches!(params.discrete(parameter), Some(ParamValue::Bool(_))))
            {
                return Err(ProcessorError::InvalidParameterDomain);
            }
        }
        Ok(())
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let position = params
            .float(Self::POSITION)
            .expect("validated pan parameter");
        let mono = params.discrete(Self::MONO) == Some(ParamValue::Bool(true));
        let swap = params.discrete(Self::SWAP) == Some(ParamValue::Bool(true));
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated stereo input");
                let mut output = outputs.get_mut(0).expect("validated stereo output");
                let mut channels = output.channels_mut();
                let left_output = channels.next().expect("validated left output");
                let right_output = channels.next().expect("validated right output");
                for (i, (left, right)) in left_output
                    .iter_mut()
                    .zip(right_output.iter_mut())
                    .enumerate()
                {
                    let (l, r) = Self::stereo(
                        input.channel(0)[i].to_f64(),
                        input.channel(1)[i].to_f64(),
                        position.sample(i),
                        mono,
                        swap,
                    );
                    *left = S::from_f64(l);
                    *right = S::from_f64(r);
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated stereo pair");
                let mut channels = block.channels_mut();
                let left = channels.next().expect("validated left channel");
                let right = channels.next().expect("validated right channel");
                for (i, (left, right)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    let (l, r) = Self::stereo(
                        left.to_f64(),
                        right.to_f64(),
                        position.sample(i),
                        mono,
                        swap,
                    );
                    *left = S::from_f64(l);
                    *right = S::from_f64(r);
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("pan requires an output"),
        }
    }
}
