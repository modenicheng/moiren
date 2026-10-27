use super::super::{ProcessContext, ProcessorError, RtProcessor};
use super::dynamics::{
    db_to_gain, float_specs, resolve_float, smooth, validate_float_domains, validate_linked_io,
};
use crate::{
    buffer::{FrameIo, PreparedIo, ProcessIo},
    control::{FloatRamp, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ProcessorId};
use moiren_signal::{ErasedBinding, LatestValue, PreparedPublisher, PublisherHandle};

/// Initial control-side values. Gains, threshold and knee are dB; times are ms.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompressorSettings {
    pub input_gain_db: f64,
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub hold_ms: f64,
    pub knee_db: f64,
    pub makeup_gain_db: f64,
    pub output_gain_db: f64,
    pub mix: f64,
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            input_gain_db: 0.0,
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            hold_ms: 0.0,
            knee_db: 6.0,
            makeup_gain_db: 0.0,
            output_gain_db: 0.0,
            mix: 1.0,
        }
    }
}

/// Feedforward peak compressor with one detector shared by all channels.
/// Persistent state occupies fixed storage independent of block/channel count.
#[derive(Debug, Default)]
pub struct Compressor {
    reduction_db: f64,
    hold_elapsed_frames: u64,
    signal: Option<CompressorSignal>,
}

struct CompressorSignal {
    publisher: PublisherHandle<LatestValue<f32>>,
    bindings: [ErasedBinding; 1],
}

impl std::fmt::Debug for CompressorSignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompressorSignal")
            .finish_non_exhaustive()
    }
}

impl Compressor {
    pub const INPUT_GAIN: ParameterId = ParameterId(0);
    pub const THRESHOLD: ParameterId = ParameterId(1);
    pub const RATIO: ParameterId = ParameterId(2);
    pub const ATTACK: ParameterId = ParameterId(3);
    pub const RELEASE: ParameterId = ParameterId(4);
    pub const HOLD: ParameterId = ParameterId(5);
    pub const KNEE: ParameterId = ParameterId(6);
    pub const MAKEUP_GAIN: ParameterId = ParameterId(7);
    pub const OUTPUT_GAIN: ParameterId = ParameterId(8);
    pub const MIX: ParameterId = ParameterId(9);

    const DOMAINS: [(f64, f64); 10] = [
        (-60.0, 60.0),
        (-120.0, 24.0),
        (1.0, 100.0),
        (0.0, 60_000.0),
        (0.0, 60_000.0),
        (0.0, 60_000.0),
        (0.0, 60.0),
        (-60.0, 60.0),
        (-60.0, 60.0),
        (0.0, 1.0),
    ];

    pub fn new() -> Self {
        Self::default()
    }

    /// Prepares optional publication of the detector's positive dB attenuation.
    pub fn with_signal(publisher: PreparedPublisher<LatestValue<f32>>) -> Self {
        Self {
            signal: Some(CompressorSignal {
                publisher: publisher.handle(),
                bindings: [publisher.erase()],
            }),
            ..Self::default()
        }
    }

    /// All ten values have bounded float domains; invalid initial settings are
    /// rejected by parameter_channel before a processor can enter render.
    pub fn parameters(id: ProcessorId, settings: CompressorSettings) -> [ParamSpec; 10] {
        let values = [
            settings.input_gain_db,
            settings.threshold_db,
            settings.ratio,
            settings.attack_ms,
            settings.release_ms,
            settings.hold_ms,
            settings.knee_db,
            settings.makeup_gain_db,
            settings.output_gain_db,
            settings.mix,
        ];
        float_specs(id, &Self::DOMAINS, values)
    }

    /// Unified preparation entry used by rack expansion and node lowering:
    /// the kernel and its parameter schema come from one call, so a prepared
    /// kernel and its table cannot drift apart. The signal-publishing variant
    /// (`with_signal`) replaces only the kernel, never this schema.
    pub fn prepare(id: ProcessorId, settings: CompressorSettings) -> (Self, Vec<ParamSpec>) {
        (Self::new(), Self::parameters(id, settings).to_vec())
    }

    /// Effective settings after applying recorded parameter intent by id. Each
    /// field is keyed by its named constant — never by declaration order — so
    /// reordering `CompressorSettings` can no longer silently misroute intent
    /// (the hazard of the positional backfill this replaces). The result feeds
    /// both the default kernel construction and [`Compressor::parameters`],
    /// keeping the bound kernel and its schema from drifting apart.
    pub(crate) fn resolve_settings(
        settings: CompressorSettings,
        mut intent: impl FnMut(ParameterId, ParamValue) -> ParamValue,
    ) -> CompressorSettings {
        CompressorSettings {
            input_gain_db: resolve_float(&mut intent, Self::INPUT_GAIN, settings.input_gain_db),
            threshold_db: resolve_float(&mut intent, Self::THRESHOLD, settings.threshold_db),
            ratio: resolve_float(&mut intent, Self::RATIO, settings.ratio),
            attack_ms: resolve_float(&mut intent, Self::ATTACK, settings.attack_ms),
            release_ms: resolve_float(&mut intent, Self::RELEASE, settings.release_ms),
            hold_ms: resolve_float(&mut intent, Self::HOLD, settings.hold_ms),
            knee_db: resolve_float(&mut intent, Self::KNEE, settings.knee_db),
            makeup_gain_db: resolve_float(&mut intent, Self::MAKEUP_GAIN, settings.makeup_gain_db),
            output_gain_db: resolve_float(&mut intent, Self::OUTPUT_GAIN, settings.output_gain_db),
            mix: resolve_float(&mut intent, Self::MIX, settings.mix),
        }
    }

    // Giannoulis / Massberg / Reiss, JAES 2012, Eq. (4):
    // https://joshreiss.github.io/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf
    fn target_reduction(level_db: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
        let above = level_db - threshold;
        let slope = 1.0 - 1.0 / ratio;
        if above <= -knee * 0.5 {
            0.0
        } else if knee > 0.0 && above < knee * 0.5 {
            slope * (above + knee * 0.5).powi(2) / (2.0 * knee)
        } else {
            slope * above
        }
    }

    fn coefficient(&mut self, peak: f64, values: [f64; 10], sr: f64) -> f64 {
        // Adding input gain in dB avoids overflowing the detector's amplitude.
        let level_db = if peak == 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * peak.log10() + values[0]
        };
        let target = Self::target_reduction(level_db, values[1], values[2], values[6]);
        if target >= self.reduction_db {
            self.reduction_db = smooth(self.reduction_db, target, values[3], sr);
            self.hold_elapsed_frames = 0;
        } else if self.hold_elapsed_frames < (values[5] * (sr / 1000.0)).ceil() as u64 {
            self.hold_elapsed_frames = self.hold_elapsed_frames.saturating_add(1);
        } else {
            self.reduction_db = smooth(self.reduction_db, target, values[4], sr);
        }
        let wet_gain = db_to_gain(values[0] - self.reduction_db + values[7]);
        ((1.0 - values[9]) + values[9] * wet_gain) * db_to_gain(values[8])
    }
    /// One kernel for both transports. In-place storage reads the value it is
    /// about to replace, so detection must complete before any linked channel
    /// of the frame is overwritten — the read fold precedes the write loop.
    fn process_linked<S: ProcessingSample, Io: FrameIo<S>>(
        &mut self,
        io: &mut Io,
        ramps: [FloatRamp; 10],
        sample_rate: f64,
    ) {
        for frame in 0..io.frames() {
            // Complete detection before overwriting any linked channel.
            let peak = (0..io.channel_count()).fold(0.0_f64, |peak, channel| {
                peak.max(io.read(channel, frame).abs())
            });
            let values = ramps.map(|ramp| ramp.sample(frame));
            let gain = self.coefficient(peak, values, sample_rate);
            for channel in 0..io.channel_count() {
                io.write(channel, frame, S::from_f64(io.read(channel, frame) * gain));
            }
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Compressor {
    fn signal_bindings(&self) -> &[ErasedBinding] {
        self.signal.as_ref().map_or(&[], |signal| &signal.bindings)
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_linked_io(io)
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        // Check presence first, so missing/type-mismatched schema has one error.
        for index in 0..10 {
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
        mut signals: crate::signal::ProcessSignals<'_>,
    ) {
        let ramps: [_; 10] = std::array::from_fn(|index| {
            params
                .float(ParameterId(index as u32))
                .expect("validated compressor parameter")
        });
        linked_io!(io, self, "compressor", ramps, ctx.processing_sr);
        if let Some(signal) = &self.signal
            && signals
                .demand(&signal.publisher)
                .is_ok_and(|demand| demand.active)
        {
            let _ = signals.publish(&signal.publisher, self.reduction_db as f32);
        }
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    use crate::{
        buffer::{BufferArena, BufferSlotLayout, PortAccess},
        control::parameter_channel,
    };
    use moiren_signal::Bus;

    #[test]
    fn signal_publishes_private_reduction_field_and_demand_never_resets_dsp() {
        fn check<S: ProcessingSample>(in_place: bool) {
            let mut bus = Bus::new(Default::default()).unwrap();
            let topic = bus
                .register_latest_value("compressor/private", 0.0_f32)
                .unwrap();
            let (control, mut host) = bus.create_host(2).unwrap();
            let key = control.new_binding().unwrap();
            let publisher = control.prepare(&topic, key).unwrap();
            let update = control.prepare_update(&[publisher.erase()]).unwrap();
            let op = update.op_bindings(key).unwrap();
            drop(host.activate(update).unwrap());
            let mut compressor = Compressor::with_signal(publisher);
            let mut baseline = Compressor::new();
            let settings = CompressorSettings {
                threshold_db: -12.0,
                knee_db: 0.0,
                attack_ms: 2.0,
                input_gain_db: 6.0,
                makeup_gain_db: 3.0,
                output_gain_db: -2.0,
                mix: 0.0,
                ..Default::default()
            };
            let (_, parameters) = parameter_channel(
                Compressor::parameters(ProcessorId(1), settings).to_vec(),
                1,
                1,
                4,
                64,
            )
            .unwrap();
            let params = parameters.bindings(ProcessorId(1));
            let mut arena = BufferArena::<S>::new(
                &[BufferSlotLayout {
                    channels: 2,
                    capacity_frames: 4,
                }; 2],
                4096,
            )
            .unwrap();
            let input = arena.slot(0).unwrap();
            let output = if in_place {
                input
            } else {
                arena.slot(1).unwrap()
            };
            let write = arena
                .prepare_io(&[PortAccess::Write {
                    port: 0,
                    slot: input,
                }])
                .unwrap();
            let ports = if in_place {
                vec![PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot: input,
                }]
            } else {
                vec![
                    PortAccess::Read {
                        port: 0,
                        slot: input,
                    },
                    PortAccess::Write {
                        port: 0,
                        slot: output,
                    },
                ]
            };
            let io = arena.prepare_io(&ports).unwrap();
            let mut subscriber = None;
            let mut last_reduction = 0.0;
            for block in 0..4 {
                if block == 1 || block == 3 {
                    subscriber = Some(bus.subscribe(&topic, Default::default()).unwrap());
                }
                if block == 2 {
                    subscriber = None;
                }
                host.begin_block();
                let ctx = ProcessContext {
                    timeline_epoch: 1,
                    timeline_start: block * 4,
                    frames: 4,
                    processing_sr: 1000.0,
                };
                for enabled in [false, true] {
                    arena
                        .with_io(&write, 0..4, |io| {
                            let ProcessIo::Separate { mut outputs, .. } = io else {
                                panic!()
                            };
                            for channel in outputs.get_mut(0).unwrap().channels_mut() {
                                channel.fill(S::from_f64(1.0));
                            }
                        })
                        .unwrap();
                    arena
                        .with_io(&io, 0..4, |io| {
                            if enabled {
                                compressor.process(
                                    &ctx,
                                    io,
                                    parameters.view(&params),
                                    host.scope(&op).into(),
                                );
                            } else {
                                baseline.process(
                                    &ctx,
                                    io,
                                    parameters.view(&params),
                                    crate::signal::ProcessSignals::empty(&mut ()),
                                );
                            }
                        })
                        .unwrap();
                }
                assert!((compressor.reduction_db - baseline.reduction_db).abs() <= 1e-12);
                assert_eq!(compressor.hold_elapsed_frames, baseline.hold_elapsed_frames);
                assert!(compressor.reduction_db > last_reduction);
                last_reduction = compressor.reduction_db;
                crate::test_support::pump(&mut bus);
                if let Some(reader) = &mut subscriber {
                    assert_eq!(
                        reader.read_latest().unwrap().value,
                        compressor.reduction_db as f32
                    );
                }
                assert_eq!(
                    topic.stats().attempts,
                    if block < 1 {
                        0
                    } else if block < 3 {
                        1
                    } else {
                        2
                    }
                );
            }
        }
        for in_place in [false, true] {
            check::<f32>(in_place);
            check::<f64>(in_place);
        }
    }
}
