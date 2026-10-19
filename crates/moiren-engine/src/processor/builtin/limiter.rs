use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::{
    db_to_gain, float_specs, sanitize_sample_rate, smooth, validate_float_domains,
    validate_linked_io,
};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParameterId, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
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
        let values = [
            settings.threshold_db,
            settings.release_ms,
            settings.input_gain_db,
        ];
        float_specs(id, &Self::DOMAINS, values)
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Limiter {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_linked_io(io)
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        for index in 0..3 {
            params
                .float(ParameterId(index))
                .ok_or(ProcessorError::MissingParameter)?;
        }
        validate_float_domains(&params, &Self::DOMAINS)
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
                    let input_gain = db_to_gain(input_gain.sample(frame));
                    let threshold = db_to_gain(threshold.sample(frame));
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
                        smooth(
                            self.gain,
                            target,
                            release.sample(frame),
                            sanitize_sample_rate(ctx.processing_sr),
                        )
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
                    let input_gain = db_to_gain(input_gain.sample(frame));
                    let threshold = db_to_gain(threshold.sample(frame));
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
                        smooth(
                            self.gain,
                            target,
                            release.sample(frame),
                            sanitize_sample_rate(ctx.processing_sr),
                        )
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
