use super::Pan;
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::{FloatRamp, ParamDomain, ParamSpec, ProcessParameters},
    processor::{ProcessContext, ProcessorError, RtProcessor},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChannelMode {
    #[default]
    Normal,
    Mono,
    Solo(usize),
    Swap,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripConfig {
    pub gain: f64,
    /// Stereo balance after the channel transform; retained but ignored for
    /// other layouts so changing a node's width does not lose its setting.
    pub balance: f64,
    pub muted: bool,
    pub mode: ChannelMode,
}

impl Default for StripConfig {
    fn default() -> Self {
        Self {
            gain: 1.,
            balance: 0.,
            muted: false,
            mode: ChannelMode::Normal,
        }
    }
}

impl StripConfig {
    pub fn valid(self, channels: usize) -> bool {
        self.gain.is_finite()
            && (0. ..=16.).contains(&self.gain)
            && self.balance.is_finite()
            && (-1. ..=1.).contains(&self.balance)
            && !matches!(self.mode, ChannelMode::Solo(channel) if channel >= channels)
    }
}

pub(crate) struct ChannelStrip {
    pub channels: usize,
    pub config: StripConfig,
}

impl ChannelStrip {
    pub const GAIN: ParameterId = ParameterId(0);
    pub const BALANCE: ParameterId = ParameterId(1);
    pub const MODE: ParameterId = ParameterId(2);
    pub const EFFECTIVE_MUTE: ParameterId = ParameterId(3);

    /// Enum index space shared with the app's `MixerState` channel mode:
    /// 0 = Normal, 1 = Mono, `2..channels+2` = Solo(n), `channels+2` = Swap.
    pub fn mode_index(mode: ChannelMode, channels: usize) -> u32 {
        match mode {
            ChannelMode::Normal => 0,
            ChannelMode::Mono => 1,
            ChannelMode::Solo(channel) => u32::try_from(channel).unwrap_or(0) + 2,
            ChannelMode::Swap => Self::mode_variants(channels) - 1,
        }
    }

    pub fn mode_from_index(index: u32, channels: usize) -> ChannelMode {
        if index == 0 {
            ChannelMode::Normal
        } else if index == 1 {
            ChannelMode::Mono
        } else if index + 1 == Self::mode_variants(channels) {
            ChannelMode::Swap
        } else {
            ChannelMode::Solo(index.saturating_sub(2) as usize)
        }
    }

    fn mode_variants(channels: usize) -> u32 {
        u32::try_from(channels)
            .unwrap_or(u32::MAX - 3)
            .saturating_add(3)
    }

    /// Live parameter specs for one strip instance. PRE-fader instances pass
    /// `include_gain = false`: their gain stays at unity by construction, so
    /// the slot is absent rather than pinned.
    pub fn parameters(
        processor: ProcessorId,
        channels: usize,
        config: StripConfig,
        include_gain: bool,
    ) -> Vec<ParamSpec> {
        let key = |parameter| ParameterKey {
            processor,
            parameter,
        };
        let mut specs = Vec::with_capacity(4);
        if include_gain {
            specs.push(ParamSpec {
                key: key(Self::GAIN),
                domain: ParamDomain::Float { min: 0., max: 16. },
                initial: ParamValue::Float(config.gain),
            });
        }
        specs.extend([
            ParamSpec {
                key: key(Self::BALANCE),
                domain: ParamDomain::Float { min: -1., max: 1. },
                initial: ParamValue::Float(config.balance),
            },
            ParamSpec {
                key: key(Self::MODE),
                domain: ParamDomain::Enum {
                    variants: Self::mode_variants(channels),
                },
                initial: ParamValue::Enum(Self::mode_index(config.mode, channels)),
            },
            ParamSpec {
                key: key(Self::EFFECTIVE_MUTE),
                domain: ParamDomain::Bool,
                initial: ParamValue::Bool(config.muted),
            },
        ]);
        specs
    }

    /// Effective config after applying recorded parameter intent by id; the
    /// result feeds both kernel construction and [`ChannelStrip::parameters`]
    /// so recompiles keep live values. `channels` is the narrowest port width
    /// the strip must satisfy. Type mismatches are unreachable — validation
    /// only records in-domain values.
    pub(crate) fn resolve(
        config: StripConfig,
        channels: usize,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> StripConfig {
        let mut resolved = config;
        if let ParamValue::Float(gain) = intent(Self::GAIN, ParamValue::Float(config.gain))
            && gain.is_finite()
            && (0. ..=16.).contains(&gain)
        {
            resolved.gain = gain;
        }
        if let ParamValue::Float(balance) = intent(Self::BALANCE, ParamValue::Float(config.balance))
            && balance.is_finite()
            && (-1. ..=1.).contains(&balance)
        {
            resolved.balance = balance;
        }
        if let ParamValue::Bool(muted) =
            intent(Self::EFFECTIVE_MUTE, ParamValue::Bool(config.muted))
        {
            resolved.muted = muted;
        }
        if let ParamValue::Enum(index) = intent(
            Self::MODE,
            ParamValue::Enum(Self::mode_index(config.mode, channels)),
        ) && index < Self::mode_variants(channels)
        {
            resolved.mode = Self::mode_from_index(index, channels);
        }
        resolved
    }
}

impl<S: ProcessingSample> RtProcessor<S> for ChannelStrip {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.channels)
            || io.output_channels(0) != Some(self.channels)
            || !self.config.valid(self.channels)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        if params.float(Self::BALANCE).is_none()
            || !matches!(
                params.domain(Self::BALANCE),
                Some(ParamDomain::Float { min, max }) if min >= -1.0 && max <= 1.0
            )
        {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        if !matches!(params.discrete(Self::MODE), Some(ParamValue::Enum(_)))
            || !matches!(
                params.domain(Self::MODE),
                Some(ParamDomain::Enum { variants }) if variants == Self::mode_variants(self.channels)
            )
        {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        if !matches!(
            params.discrete(Self::EFFECTIVE_MUTE),
            Some(ParamValue::Bool(_))
        ) || params.domain(Self::EFFECTIVE_MUTE) != Some(ParamDomain::Bool)
        {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        // Absent on PRE-fader instances; present it must use the gain schema.
        if let Some(domain) = params.domain(Self::GAIN)
            && (!matches!(
                domain,
                ParamDomain::Float { min, max } if min >= 0.0 && max <= 16.0
            ) || params.float(Self::GAIN).is_none())
        {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        Ok(())
    }

    fn process(
        &mut self,
        _: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
        _: crate::signal::ProcessSignals<'_>,
    ) {
        if let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        {
            let input = inputs.get(0).expect("prepared strip input");
            let mut output = outputs.get_mut(0).expect("prepared strip output");
            if params.discrete(Self::EFFECTIVE_MUTE) == Some(ParamValue::Bool(true)) {
                for samples in output.channels_mut() {
                    samples.fill(S::ZERO);
                }
                return;
            }
            let balance = params
                .float(Self::BALANCE)
                .expect("validated strip balance");
            // PRE-fader instances have no gain slot; unity by construction.
            let gain = params
                .float(Self::GAIN)
                .unwrap_or_else(|| FloatRamp::constant(1.));
            let mode = match params.discrete(Self::MODE) {
                Some(ParamValue::Enum(index)) => Self::mode_from_index(index, self.channels),
                _ => unreachable!("validated strip mode"),
            };
            for (channel, samples) in output.channels_mut().enumerate() {
                // Share the send/Pan law, but apply it after this strip's
                // monitoring transform. PRE uses the same kernel at unity gain.
                for (frame, sample) in samples.iter_mut().enumerate() {
                    let value = match mode {
                        ChannelMode::Normal => input.channel(channel)[frame].to_f64(),
                        ChannelMode::Swap => {
                            input.channel(self.channels - channel - 1)[frame].to_f64()
                        }
                        ChannelMode::Solo(selected) => input.channel(selected)[frame].to_f64(),
                        ChannelMode::Mono => input
                            .channels()
                            .map(|samples| samples[frame].to_f64() / self.channels as f64)
                            .sum(),
                    };
                    let coefficient = if self.channels == 2 {
                        Pan::coefficient(channel, balance.sample(frame))
                    } else {
                        1.
                    };
                    *sample = S::from_f64(value * coefficient * gain.sample(frame));
                }
            }
        }
    }
}
