use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::{
    db_to_gain, float_specs, resolve_float, sanitize_sample_rate, smooth, validate_float_domains,
    validate_linked_io,
};
use crate::{
    buffer::{FrameIo, InPlaceFrames, PreparedIo, ProcessIo, SeparateFrames},
    control::{FloatRamp, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ProcessorId};

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

    /// Unified preparation entry used by rack expansion and node lowering:
    /// the kernel and its parameter schema come from one call, so a prepared
    /// kernel and its table cannot drift apart.
    pub fn prepare(id: ProcessorId, settings: LimiterSettings) -> (Self, Vec<ParamSpec>) {
        (Self::new(), Self::parameters(id, settings).to_vec())
    }

    /// Effective settings after applying recorded parameter intent by id; the
    /// result feeds both the default kernel construction and
    /// [`Limiter::parameters`].
    pub(crate) fn resolve_settings(
        settings: LimiterSettings,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> LimiterSettings {
        LimiterSettings {
            threshold_db: resolve_float(&mut intent, Self::THRESHOLD, settings.threshold_db),
            release_ms: resolve_float(&mut intent, Self::RELEASE, settings.release_ms),
            input_gain_db: resolve_float(&mut intent, Self::INPUT_GAIN, settings.input_gain_db),
        }
    }
    /// One kernel for both transports: the linked detector reads the whole
    /// input frame before any channel of it is written.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(
        &mut self,
        io: &mut Io,
        threshold: FloatRamp,
        release: FloatRamp,
        input_gain: FloatRamp,
        sample_rate: f64,
    ) {
        let sample_rate = sanitize_sample_rate(sample_rate);
        for frame in 0..io.frames() {
            let input_gain = db_to_gain(input_gain.sample(frame));
            let threshold = db_to_gain(threshold.sample(frame));
            let peak = (0..io.channel_count()).fold(0.0_f64, |peak, channel| {
                peak.max(io.read(channel, frame).abs() * input_gain)
            });
            let target = if peak > threshold {
                threshold / peak
            } else {
                1.0
            };
            self.gain = if target < self.gain {
                target
            } else {
                smooth(self.gain, target, release.sample(frame), sample_rate)
            };
            for channel in 0..io.channel_count() {
                io.write(
                    channel,
                    frame,
                    S::from_f64(io.read(channel, frame) * input_gain * self.gain),
                );
            }
        }
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
            } => self.process_linked(
                &mut SeparateFrames::new(
                    inputs.get(0).expect("validated limiter input"),
                    outputs.get_mut(0).expect("validated limiter output"),
                ),
                threshold,
                release,
                input_gain,
                ctx.processing_sr,
            ),
            ProcessIo::InPlace { mut pairs, .. } => self.process_linked(
                &mut InPlaceFrames::new(pairs.get_mut(0, 0).expect("validated limiter pair")),
                threshold,
                release,
                input_gain,
                ctx.processing_sr,
            ),
            ProcessIo::ReadOnly { .. } => unreachable!("limiter requires an output"),
        }
    }
}
