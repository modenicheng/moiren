use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::resolve_float;
use crate::{
    buffer::{FrameIo, InPlaceFrames, PreparedIo, ProcessIo, SeparateFrames},
    control::{FloatRamp, ParamDomain, ParamSpec, ProcessParameters},
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

    /// Effective (position, mono, swap) after applying recorded parameter
    /// intent by id; the result feeds both the default kernel construction and
    /// [`Pan::parameters`]. A type mismatch is unreachable — validation only
    /// records Bool values for the switches.
    pub(crate) fn resolve(
        position: f64,
        mono: bool,
        swap: bool,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> (f64, bool, bool) {
        let position = resolve_float(&mut intent, Self::POSITION, position);
        let mono = match intent(Self::MONO, ParamValue::Bool(mono)) {
            ParamValue::Bool(value) => value,
            _ => unreachable!("validated boolean parameter"),
        };
        let swap = match intent(Self::SWAP, ParamValue::Bool(swap)) {
            ParamValue::Bool(value) => value,
            _ => unreachable!("validated boolean parameter"),
        };
        (position, mono, swap)
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

    /// One kernel for both transports: both input channels of a frame are
    /// read before either is written.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(
        io: &mut Io,
        position: FloatRamp,
        mono: bool,
        swap: bool,
    ) {
        for frame in 0..io.frames() {
            let (left, right) = Self::stereo(
                io.read(0, frame),
                io.read(1, frame),
                position.sample(frame),
                mono,
                swap,
            );
            io.write(0, frame, S::from_f64(left));
            io.write(1, frame, S::from_f64(right));
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Pan {
    fn transport_interchangeable(&self) -> bool {
        // Stateless per-frame balance: both transports produce the same
        // samples and leave no persistent state.
        true
    }
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
            } => Self::process_linked(
                &mut SeparateFrames::new(
                    inputs.get(0).expect("validated stereo input"),
                    outputs.get_mut(0).expect("validated stereo output"),
                ),
                position,
                mono,
                swap,
            ),
            ProcessIo::InPlace { mut pairs, .. } => Self::process_linked(
                &mut InPlaceFrames::new(pairs.get_mut(0, 0).expect("validated stereo pair")),
                position,
                mono,
                swap,
            ),
            ProcessIo::ReadOnly { .. } => unreachable!("pan requires an output"),
        }
    }
}
