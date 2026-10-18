use super::*;
use moiren_engine::meter::signal::{AudioWindow, ChannelLevel, MeterError, SignalLevelMeter};
use moiren_signal::{BlockInit, Bus, PumpBudget};

fn window() -> AudioWindow {
    AudioWindow {
        timeline_epoch: 3,
        frame_start: 0,
        frame_end: 4,
        sample_rate: 48_000.0,
    }
}

fn topic(
    bus: &mut Bus,
    capacity: usize,
) -> moiren_signal::Topic<moiren_signal::LatestBlock<AudioWindow, ChannelLevel>> {
    bus.register_latest_block(
        "measurement/levels",
        BlockInit {
            header: window(),
            element: ChannelLevel {
                peak: 0.0,
                rms: 0.0,
            },
            capacity,
        },
    )
    .unwrap()
}

#[test]
fn meter_rejects_invalid_windows_channels_and_exact_capacity_mismatches() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = topic(&mut bus, 2);
    let (control, _host) = bus.create_host(2).unwrap();
    for (channels, frames, expected) in [
        (0, 4, MeterError::InvalidChannels),
        (2, 0, MeterError::InvalidWindow),
        (1, 4, MeterError::CapacityMismatch),
        (3, 4, MeterError::CapacityMismatch),
    ] {
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        assert!(
            matches!(SignalLevelMeter::new(channels, frames, publisher), Err(error) if error == expected)
        );
    }
}

struct StereoSource;
impl<S: ProcessingSample> RtAudioSource<S> for StereoSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        ctx: &ProcessContext,
        mut output: moiren_engine::buffer::AudioBlockMut<'_, S>,
    ) -> BoundaryReport {
        for frame in 0..ctx.frames {
            let position = ((ctx.timeline_start + frame as u64) % 4) as usize;
            output.channel_mut(0)[frame] = S::from_f64([1.0, 0.0, -1.0, 0.0][position]);
            output.channel_mut(1)[frame] = S::from_f64(0.5);
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..Default::default()
        }
    }
}

fn render<S: ProcessingSample>(
    segmented: bool,
    observed: bool,
) -> (AudioWindow, [ChannelLevel; 2], [f64; 8]) {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = topic(&mut bus, 2);
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (host_control, host) = bus.create_host(2).unwrap();
    let publisher = host_control
        .prepare(&topic, host_control.new_binding().unwrap())
        .unwrap();
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, gain, SendParams::default());
    connect(&mut graph, gain, sink, SendParams::default());
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, StereoSource).unwrap();
    bindings.bind_gain(gain, 1.0).unwrap();
    if observed {
        bindings
            .observe_output(
                output(&graph, source),
                SignalLevelMeter::new(2, 4, publisher).unwrap(),
            )
            .unwrap();
    }
    let (writer, mut reader) = audio_bridge::<S>(2, 8, 4096).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile_with_signals(&graph, bindings, config(), &host_control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    if segmented {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: 1,
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target: ParameterKey {
                            processor: compiled.bindings.node(gain).unwrap(),
                            parameter: Gain::LEVEL
                        },
                        at: ApplyAt::Frame(1),
                        value: ParamValue::Float(1.0),
                        ramp_frames: 0,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    let (report, allocations) =
        allocation::track_allocations(|| compiled.engine.render(4).unwrap());
    assert_eq!(allocations, (0, 0));
    assert_eq!(report.segments, if segmented { 2 } else { 1 });
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 128,
        copy_bytes: 65536,
    });
    let mut samples = [S::ZERO; 8];
    reader.read_interleaved(&mut samples).unwrap();
    let samples = samples.map(ProcessingSample::to_f64);
    if observed {
        let publication = subscriber.read_latest().unwrap();
        assert_eq!(publication.elements().len(), 2);
        (
            *publication.header(),
            publication.elements().try_into().unwrap(),
            samples,
        )
    } else {
        assert!(subscriber.read_latest().is_none());
        (
            window(),
            [ChannelLevel {
                peak: 0.0,
                rms: 0.0,
            }; 2],
            samples,
        )
    }
}

#[test]
fn per_channel_exact_window_matches_parameter_segments_and_preserves_audio() {
    fn check<S: ProcessingSample>() {
        let whole = render::<S>(false, true);
        let segmented = render::<S>(true, true);
        assert_eq!(whole, segmented);
        assert_eq!(whole.0, window());
        assert_eq!(
            whole.1[0],
            ChannelLevel {
                peak: 1.0,
                rms: 0.5_f32.sqrt()
            }
        );
        assert_eq!(
            whole.1[1],
            ChannelLevel {
                peak: 0.5,
                rms: 0.5
            }
        );
        assert_eq!(whole.2, [1.0, 0.5, 0.0, 0.5, -1.0, 0.5, 0.0, 0.5]);
        assert_eq!(render::<S>(true, false).2, whole.2);
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn compressor_signal_binding_errors_are_rejected_during_preparation() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_latest_value("measurement/errors", 0.0_f32)
        .unwrap();
    let (control, _host) = bus.create_host(2).unwrap();
    let (foreign, _foreign_host) = bus.create_host(2).unwrap();
    let mut graph = LogicalGraph::new();
    let compressor = graph.create_node(NodeKind::Compressor, 2).unwrap();
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_compressor_with_signal(
            compressor,
            Default::default(),
            control
                .prepare(&topic, control.new_binding().unwrap())
                .unwrap(),
        )
        .unwrap();
    assert!(
        matches!(bindings.bind_compressor(compressor, Default::default()), Err(CompileError::DuplicateBinding { node }) if node == compressor)
    );
    assert!(matches!(
        compile(&graph, bindings, config()),
        Err(CompileError::SignalsRequireHost)
    ));
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_compressor_with_signal(
            compressor,
            Default::default(),
            control
                .prepare(&topic, control.new_binding().unwrap())
                .unwrap(),
        )
        .unwrap();
    assert!(compile_with_signals(&graph, bindings, config(), &foreign).is_err());
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_compressor_with_signal(
            compressor,
            CompressorSettings {
                ratio: 0.0,
                ..Default::default()
            },
            control
                .prepare(&topic, control.new_binding().unwrap())
                .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        compile_with_signals(&graph, bindings, config(), &control),
        Err(CompileError::Control(_))
    ));
}

#[test]
fn compiler_binds_optional_compressor_signal_without_changing_samples() {
    let mut rendered = Vec::new();
    for publish in [false, true] {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_value("measurement/reduction", 0.0_f32)
            .unwrap();
        let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, host) = bus.create_host(2).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let compressor = graph.create_node(NodeKind::Compressor, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        connect(&mut graph, source, compressor, SendParams::default());
        connect(&mut graph, compressor, sink, SendParams::default());
        let settings = CompressorSettings {
            threshold_db: -12.0,
            attack_ms: 0.0,
            knee_db: 0.0,
            ..Default::default()
        };
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 1.0,
                },
            )
            .unwrap();
        if publish {
            bindings
                .bind_compressor_with_signal(compressor, settings, publisher)
                .unwrap();
        } else {
            bindings.bind_compressor(compressor, settings).unwrap();
        }
        let (writer, mut audio) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile_with_signals(&graph, bindings, config(), &control).unwrap();
        compiled.engine.attach_signal_host(host).unwrap();
        let (_, counts) = allocation::track_allocations(|| compiled.engine.render(4).unwrap());
        assert_eq!(counts, (0, 0));
        bus.pump(PumpBudget {
            topic_checks: 1024,
            deliveries: 128,
            copy_bytes: 65536,
        });
        if publish {
            assert_eq!(reader.read_latest().unwrap().value, 9.0);
        } else {
            assert!(reader.read_latest().is_none());
        }
        let mut samples = [0.0; 8];
        audio.read_interleaved(&mut samples).unwrap();
        rendered.push(samples);
    }
    assert_eq!(rendered[0].len(), rendered[1].len());
    for (without_signal, with_signal) in rendered[0].iter().zip(&rendered[1]) {
        assert!((without_signal - with_signal).abs() <= 1e-12);
    }
}
