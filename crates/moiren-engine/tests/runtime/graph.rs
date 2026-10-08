use super::{
    allocation::track_allocations,
    support::{EPOCH, REV, SINK, SOURCE, pipeline, request},
};
use moiren_core::protocol::{
    ApplyAt, ParamValue, ParameterKey, ParameterRequest, ProcessorId, ReplyCode,
};
use moiren_engine::{
    boundary::{ConstantSource, SinkAdapter, SourceAdapter, audio_bridge},
    buffer::{BufferArena, BufferSlotLayout, PortAccess},
    control::parameter_channel,
    processor::{Bus, Pan},
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
};

#[test]
fn compiler_prepared_graph_renders_without_allocations_or_deallocations() {
    use moiren_core::graph::*;
    use moiren_engine::compiler::*;
    fn check<S: moiren_engine::sample::ProcessingSample>() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let mut edges = Vec::new();
        for _ in 0..2 {
            let input = graph.add_input_port(bus, 2).unwrap();
            edges.push(
                graph
                    .connect(
                        graph.get_node(source).unwrap().outputs()[0].id(),
                        input,
                        SendParams::default(),
                    )
                    .unwrap(),
            );
        }
        graph
            .connect(
                graph.get_node(bus).unwrap().outputs()[0].id(),
                graph.get_node(sink).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
        let (writer, mut reader) = audio_bridge::<S>(2, 32, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 0.25,
                },
            )
            .unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: 48000.0,
                    max_block_frames: 8,
                    max_events_per_block: 8,
                },
                audio_byte_budget: 4096,
                plan_revision: REV,
                timeline_epoch: EPOCH,
                control_capacity: 8,
                control_horizon_frames: 48000,
            },
        )
        .unwrap();
        let keys = compiled.bindings.edge(edges[0]).unwrap();
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: 1,
                        plan_revision: REV,
                        timeline_epoch: EPOCH,
                        target: keys.pan,
                        at: ApplyAt::Frame(1),
                        value: ParamValue::Float(1.0),
                        ramp_frames: 4,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
        let (results, counts) = track_allocations(|| {
            [
                compiled.engine.render(3),
                compiled.engine.render(1),
                compiled.engine.render(4),
            ]
        });
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(counts, (0, 0));
        let mut samples = [S::ZERO; 16];
        assert_eq!(
            reader
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            8
        );
    }
    check::<f32>();
    check::<f64>();
}

fn bus_and_pan_render<S: moiren_engine::sample::ProcessingSample>(in_place: bool) {
    let second_source = ProcessorId(11);
    let bus = ProcessorId(12);
    let pan = ProcessorId(13);
    let raw_sink = ProcessorId(14);
    let (mut input_a, source_a) = audio_bridge::<S>(2, 16, 4096).unwrap();
    let (mut input_b, source_b) = audio_bridge::<S>(2, 16, 4096).unwrap();
    let (sink, mut output) = audio_bridge::<S>(2, 16, 4096).unwrap();
    let (raw, mut raw_output) = audio_bridge::<S>(2, 16, 4096).unwrap();
    let resources = RtResources::new(vec![
        ProcessorInstance::new(SOURCE, SourceAdapter(source_a)),
        ProcessorInstance::new(second_source, SourceAdapter(source_b)),
        ProcessorInstance::new(bus, Bus),
        ProcessorInstance::new(pan, Pan),
        ProcessorInstance::new(raw_sink, SinkAdapter(raw)),
        ProcessorInstance::new(SINK, SinkAdapter(sink)),
    ])
    .unwrap();
    let (mut control, parameters) =
        parameter_channel(vec![Pan::parameter(pan, -1.0)], REV, EPOCH, 4, 1024).unwrap();
    let event = ParameterRequest {
        request_id: 100,
        plan_revision: REV,
        timeline_epoch: EPOCH,
        target: ParameterKey {
            processor: pan,
            parameter: Pan::POSITION,
        },
        at: ApplyAt::Frame(2),
        value: ParamValue::Float(1.0),
        ramp_frames: 4,
    };
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Float(1.1),
                    ..event
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(control.submit(event, 0).code, ReplyCode::Accepted);
    let arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 8,
        }; 4],
        4096,
    )
    .unwrap();
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let mixed = arena.slot(2).unwrap();
    let balanced = if in_place {
        mixed
    } else {
        arena.slot(3).unwrap()
    };
    let specs = vec![
        OpSpec {
            processor: SOURCE,
            io: arena
                .prepare_io(&[PortAccess::Write { port: 0, slot: a }])
                .unwrap(),
        },
        OpSpec {
            processor: second_source,
            io: arena
                .prepare_io(&[PortAccess::Write { port: 0, slot: b }])
                .unwrap(),
        },
        OpSpec {
            processor: bus,
            io: arena
                .prepare_io(&[
                    PortAccess::Read { port: 0, slot: a },
                    PortAccess::Read { port: 7, slot: a },
                    PortAccess::Read { port: 42, slot: b },
                    PortAccess::Write {
                        port: 0,
                        slot: mixed,
                    },
                ])
                .unwrap(),
        },
        // Read the pre-pan fan-out before the last-use in-place mutation.
        OpSpec {
            processor: raw_sink,
            io: arena
                .prepare_io(&[PortAccess::Read {
                    port: 0,
                    slot: mixed,
                }])
                .unwrap(),
        },
        OpSpec {
            processor: pan,
            io: arena
                .prepare_io(&if in_place {
                    vec![PortAccess::InPlace {
                        input: 0,
                        output: 0,
                        slot: mixed,
                    }]
                } else {
                    vec![
                        PortAccess::Read {
                            port: 0,
                            slot: mixed,
                        },
                        PortAccess::Write {
                            port: 0,
                            slot: balanced,
                        },
                    ]
                })
                .unwrap(),
        },
        OpSpec {
            processor: SINK,
            io: arena
                .prepare_io(&[PortAccess::Read {
                    port: 0,
                    slot: balanced,
                }])
                .unwrap(),
        },
    ];
    let plan = ExecutionPlan::prepare(
        arena,
        specs,
        &resources,
        &parameters,
        EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 4,
        },
    )
    .unwrap();
    let mut engine = Engine::new(plan, resources, parameters).unwrap();
    let mut a_samples = [S::ZERO; 16];
    let mut b_samples = [S::ZERO; 16];
    for frame in a_samples.as_chunks_mut::<2>().0 {
        frame[0] = S::from_f64(0.25);
        frame[1] = S::from_f64(-0.5);
    }
    for frame in b_samples.as_chunks_mut::<2>().0 {
        frame[0] = S::from_f64(0.125);
        frame[1] = S::from_f64(0.25);
    }
    let mut balanced_samples = [S::ZERO; 18];
    let mut raw_samples = [S::ZERO; 18];
    let ((filled_a, filled_b, reports, drained, raw_drained), counts) = track_allocations(|| {
        let filled_a = input_a.write_interleaved(&a_samples);
        let filled_b = input_b.write_interleaved(&b_samples);
        // The ramp starts at frame 2, crosses center, spans the 3/1/4 blocks,
        // and the last block also exercises fresh Bus silence after input exhaustion.
        let reports = [
            engine.render(3),
            engine.render(1),
            engine.render(4),
            engine.render(1),
        ];
        let drained = output.read_interleaved(&mut balanced_samples);
        let raw_drained = raw_output.read_interleaved(&mut raw_samples);
        (filled_a, filled_b, reports, drained, raw_drained)
    });
    assert_eq!(counts, (0, 0));
    assert_eq!(filled_a.unwrap().transferred_frames, 8);
    assert_eq!(filled_b.unwrap().transferred_frames, 8);
    assert_eq!(reports[0].as_ref().unwrap().segments, 2);
    for report in reports {
        report.unwrap();
    }
    assert_eq!(drained.unwrap().transferred_frames, 9);
    assert_eq!(raw_drained.unwrap().transferred_frames, 9);
    let expected = [
        0.625, 0.0, 0.625, 0.0, 0.625, -0.375, 0.625, -0.75, 0.3125, -0.75, 0.0, -0.75, 0.0, -0.75,
        0.0, -0.75, 0.0, 0.0,
    ];
    for (sample, expected) in balanced_samples.into_iter().zip(expected) {
        assert_eq!(sample.to_f64(), expected);
    }
    for frame in raw_samples[..16].as_chunks::<2>().0 {
        assert_eq!(frame[0].to_f64(), 0.625);
        assert_eq!(frame[1].to_f64(), -0.75);
    }
    assert!(
        raw_samples[16..]
            .iter()
            .all(|sample| sample.to_f64() == 0.0)
    );
    let reply = control.poll_applied().unwrap();
    assert_eq!(
        (reply.request_id, reply.code, reply.effective_frame),
        (100, ReplyCode::Applied, 2)
    );
    drop(engine.into_parts());
}

#[test]
fn bus_and_pan_handle_repeated_inputs_fan_out_and_cross_block_automation_without_allocations() {
    for in_place in [false, true] {
        bus_and_pan_render::<f32>(in_place);
        bus_and_pan_render::<f64>(in_place);
    }
}

#[test]
fn builtins_render_without_allocation_or_deallocation_even_with_full_telemetry() {
    let (mut engine, mut control, mut meter, _) = pipeline(true, 16, 16);
    for frame in 1..=8 {
        control.submit(request(frame, ApplyAt::Frame(frame), 2.0, 16), 0);
    }
    let (result, counts) = track_allocations(|| engine.render(16));
    result.unwrap();
    assert_eq!(counts, (0, 0));
    assert!(meter.latest().is_some());
    engine.render(16).unwrap();
    assert!(meter.latest().unwrap().dropped_snapshots > 0);
}
