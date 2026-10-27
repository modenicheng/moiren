use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::{
    db_to_gain, float_specs, resolve_float, sanitize_sample_rate, smooth, validate_float_domains,
    validate_linked_io,
};
use crate::{
    buffer::{FrameIo, InPlaceFrames, PreparedIo, ProcessIo, SeparateFrames},
    control::{ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ProcessorId};

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GateSettings {
    pub threshold_db: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub hold_ms: f64,
    pub range_db: f64,
}

impl Default for GateSettings {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            attack_ms: 1.0,
            release_ms: 100.0,
            hold_ms: 10.0,
            range_db: -80.0,
        }
    }
}

/// Linked peak gate with a fixed state budget independent of the channel count.
#[derive(Debug)]
pub struct Gate {
    envelope: f64,
    gain: f64,
    hold_elapsed_frames: u64,
    open: bool,
    initialized: bool,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            envelope: 0.0,
            gain: 1.0,
            hold_elapsed_frames: 0,
            open: false,
            initialized: false,
        }
    }
}

impl Gate {
    pub const THRESHOLD: ParameterId = ParameterId(0);
    pub const ATTACK: ParameterId = ParameterId(1);
    pub const RELEASE: ParameterId = ParameterId(2);
    pub const HOLD: ParameterId = ParameterId(3);
    pub const RANGE: ParameterId = ParameterId(4);

    const DOMAINS: [(f64, f64); 5] = [
        (-120.0, 0.0),
        (0.0, 5_000.0),
        (0.0, 5_000.0),
        (0.0, 5_000.0),
        (-120.0, 0.0),
    ];

    pub fn new() -> Self {
        Self::default()
    }

    pub fn parameters(id: ProcessorId, settings: GateSettings) -> [ParamSpec; 5] {
        let values = [
            settings.threshold_db,
            settings.attack_ms,
            settings.release_ms,
            settings.hold_ms,
            settings.range_db,
        ];
        float_specs(id, &Self::DOMAINS, values)
    }

    /// Unified preparation entry used by rack expansion and node lowering:
    /// the kernel and its parameter schema come from one call, so a prepared
    /// kernel and its table cannot drift apart.
    pub fn prepare(id: ProcessorId, settings: GateSettings) -> (Self, Vec<ParamSpec>) {
        (Self::new(), Self::parameters(id, settings).to_vec())
    }

    /// Effective settings after applying recorded parameter intent by id; the
    /// result feeds both the default kernel construction and [`Gate::parameters`].
    pub(crate) fn resolve_settings(
        settings: GateSettings,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> GateSettings {
        GateSettings {
            threshold_db: resolve_float(&mut intent, Self::THRESHOLD, settings.threshold_db),
            attack_ms: resolve_float(&mut intent, Self::ATTACK, settings.attack_ms),
            release_ms: resolve_float(&mut intent, Self::RELEASE, settings.release_ms),
            hold_ms: resolve_float(&mut intent, Self::HOLD, settings.hold_ms),
            range_db: resolve_float(&mut intent, Self::RANGE, settings.range_db),
        }
    }

    fn process_frame(&mut self, peak: f64, settings: GateSettings, sample_rate: f64) -> f64 {
        let GateSettings {
            threshold_db,
            attack_ms,
            release_ms,
            hold_ms,
            range_db,
        } = settings;
        let sample_rate = sanitize_sample_rate(sample_rate);
        if !self.initialized {
            self.gain = db_to_gain(range_db);
            self.initialized = true;
        }
        if peak >= self.envelope {
            self.envelope = smooth(self.envelope, peak, attack_ms, sample_rate);
        } else {
            self.envelope = smooth(self.envelope, peak, release_ms, sample_rate);
        }
        let threshold = db_to_gain(threshold_db);
        if self.envelope >= threshold {
            self.open = true;
            self.hold_elapsed_frames = 0;
        } else if self.open {
            let hold_frames = (hold_ms * sample_rate / 1_000.0).ceil() as u64;
            if self.hold_elapsed_frames < hold_frames {
                self.hold_elapsed_frames = self.hold_elapsed_frames.saturating_add(1);
            } else {
                self.open = false;
            }
        }
        let target = if self.open { 1.0 } else { db_to_gain(range_db) };
        let time = if target > self.gain {
            attack_ms
        } else {
            release_ms
        };
        self.gain = smooth(self.gain, target, time, sample_rate);
        self.gain
    }
    /// One kernel for both transports: the detector reads the whole input
    /// frame before any channel of it is written.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(
        &mut self,
        io: &mut Io,
        params: &ProcessParameters<'_>,
        sample_rate: f64,
    ) {
        let threshold = params
            .float(Self::THRESHOLD)
            .expect("validated gate threshold");
        let attack = params.float(Self::ATTACK).expect("validated gate attack");
        let release = params.float(Self::RELEASE).expect("validated gate release");
        let hold = params.float(Self::HOLD).expect("validated gate hold");
        let range = params.float(Self::RANGE).expect("validated gate range");
        for frame in 0..io.frames() {
            let peak = (0..io.channel_count()).fold(0.0_f64, |peak, channel| {
                peak.max(io.read(channel, frame).abs())
            });
            let gain = self.process_frame(
                peak,
                GateSettings {
                    threshold_db: threshold.sample(frame),
                    attack_ms: attack.sample(frame),
                    release_ms: release.sample(frame),
                    hold_ms: hold.sample(frame),
                    range_db: range.sample(frame),
                },
                sample_rate,
            );
            for channel in 0..io.channel_count() {
                io.write(channel, frame, S::from_f64(io.read(channel, frame) * gain));
            }
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Gate {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_linked_io(io)
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        for index in 0..5 {
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
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => self.process_linked(
                &mut SeparateFrames::new(
                    inputs.get(0).expect("validated gate input"),
                    outputs.get_mut(0).expect("validated gate output"),
                ),
                &params,
                ctx.processing_sr,
            ),
            ProcessIo::InPlace { mut pairs, .. } => self.process_linked(
                &mut InPlaceFrames::new(pairs.get_mut(0, 0).expect("validated gate pair")),
                &params,
                ctx.processing_sr,
            ),
            ProcessIo::ReadOnly { .. } => unreachable!("gate requires an output"),
        }
    }
}
