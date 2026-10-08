use moiren_core::protocol::*;
use moiren_engine::node::*;
use moiren_engine::{boundary::*, buffer::*, control::*, meter::*, processor::*, runtime::*};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

// Counts only this test thread while render is active; unrelated parallel test
// allocations cannot contaminate the result. TLS has constant initialization.
struct CountingAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a + 1, d));
                });
            }
        });
        // SAFETY: forwarding the caller's allocation contract unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a, d + 1));
                });
            }
        });
        // SAFETY: ptr/layout originated from this allocator's System call.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

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
        COUNTS.with(|counts| counts.set((0, 0)));
        TRACK.with(|track| track.set(true));
        let results = [
            compiled.engine.render(3),
            compiled.engine.render(1),
            compiled.engine.render(4),
        ];
        TRACK.with(|track| track.set(false));
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(COUNTS.with(Cell::get), (0, 0));
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

const REV: u64 = 7;
const EPOCH: u64 = 3;
const SOURCE: ProcessorId = ProcessorId(10);
const GAIN: ProcessorId = ProcessorId(20);
const METER: ProcessorId = ProcessorId(30);
const SINK: ProcessorId = ProcessorId(40);

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
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
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
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
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
fn io_nodes_and_bridges_render_without_allocations_with_fan_out_and_full_queues() {
    let (mut input, source) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let (sink_a, mut output_a) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let (sink_b, mut output_b) = audio_bridge::<f64>(2, 3, 4096).unwrap();
    let (input_node, mut input_reports) = InputNode::new(
        InputConfig {
            source: InputSource::Software {
                name: "allocation input".into(),
            },
            channels: 2,
            device: DeviceOptions::default(),
        },
        source,
        1,
    )
    .unwrap();
    let output_config = OutputConfig {
        target: OutputTarget::Software {
            name: "allocation output".into(),
        },
        channels: 2,
        device: DeviceOptions::default(),
    };
    let (node_a, mut reports_a) = OutputNode::new(output_config.clone(), sink_a, 1).unwrap();
    let (node_b, mut reports_b) = OutputNode::new(output_config, sink_b, 1).unwrap();
    let second_sink = ProcessorId(50);
    let resources = RtResources::new(vec![
        ProcessorInstance::new(SOURCE, input_node),
        ProcessorInstance::new(GAIN, Gain),
        ProcessorInstance::new(SINK, node_a),
        ProcessorInstance::new(second_sink, node_b),
    ])
    .unwrap();
    let (mut control, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 0.5)], REV, EPOCH, 4, 1024).unwrap();
    assert_eq!(
        control
            .submit(request(1, ApplyAt::Frame(2), 1.0, 2), 0)
            .code,
        ReplyCode::Accepted
    );
    let arena = BufferArena::<f64>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 8,
        }],
        4096,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let specs = vec![
        OpSpec {
            processor: SOURCE,
            io: arena
                .prepare_io(&[PortAccess::Write { port: 0, slot }])
                .unwrap(),
        },
        OpSpec {
            processor: GAIN,
            io: arena
                .prepare_io(&[PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot,
                }])
                .unwrap(),
        },
        OpSpec {
            processor: SINK,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap(),
        },
        OpSpec {
            processor: second_sink,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap(),
        },
    ];
    let plan = ExecutionPlan::prepare(
        arena,
        specs,
        &resources,
        &params,
        EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 4,
        },
    )
    .unwrap();
    let mut engine = Engine::new(plan, resources, params).unwrap();
    let mut a = [99.0; 16];
    let mut b = [99.0; 6];
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    let filled =
        input.write_interleaved(&[1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
    let renders = [engine.render(4), engine.render(3), engine.render(1)];
    let drained_a = output_a.read_interleaved(&mut a);
    let drained_b = output_b.read_interleaved(&mut b);
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(filled.unwrap().transferred_frames, 6);
    assert_eq!(renders[0].as_ref().unwrap().segments, 2);
    for render in renders {
        render.unwrap();
    }
    assert_eq!(drained_a.unwrap().transferred_frames, 8);
    assert_eq!(drained_b.unwrap().transferred_frames, 3);
    assert_eq!(
        a,
        [
            0.5, 1.0, 0.5, 1.0, 0.75, 1.5, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0
        ]
    );
    assert_eq!(b, a[..6]);
    input_reports.latest();
    reports_a.latest();
    reports_b.latest();
    engine.render(1).unwrap();
    let input_status = input_reports.latest().unwrap();
    assert_eq!(input_status.total_transferred_frames, 6);
    assert_eq!(input_status.total_shortfall_frames, 3);
    assert!(input_status.dropped_snapshots > 0);
    assert_eq!(reports_a.latest().unwrap().total_xruns, 0);
    assert_eq!(reports_b.latest().unwrap().total_xruns, 3);
}

struct Capture {
    samples: Arc<[AtomicU64]>,
}
impl RtAudioSink<f64> for Capture {
    fn channel_count(&self) -> usize {
        1
    }
    fn write(&mut self, ctx: &ProcessContext, input: AudioBlock<'_, f64>) -> BoundaryReport {
        for (i, sample) in input.channel(0).iter().enumerate() {
            self.samples[ctx.timeline_start as usize + i]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}
fn request(id: u64, at: ApplyAt, value: f64, ramp_frames: u32) -> ParameterRequest {
    ParameterRequest {
        request_id: id,
        plan_revision: REV,
        timeline_epoch: EPOCH,
        target: ParameterKey {
            processor: GAIN,
            parameter: Gain::LEVEL,
        },
        at,
        value: ParamValue::Float(value),
        ramp_frames,
    }
}
fn config(events: usize) -> EngineConfig {
    EngineConfig {
        processing_sr: 48000.0,
        max_block_frames: 16,
        max_events_per_block: events,
    }
}
fn pipeline(
    in_place: bool,
    events: usize,
    queue: usize,
) -> (Engine<f64>, ControlPort, MeterReader, Arc<[AtomicU64]>) {
    let (control, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, queue, 1024).unwrap();
    let (meter, reader) = level_meter(1, 1).unwrap();
    let samples: Arc<[AtomicU64]> = (0..256)
        .map(|_| AtomicU64::new(f64::NAN.to_bits()))
        .collect::<Vec<_>>()
        .into();
    let resources = RtResources::new(vec![
        ProcessorInstance::new(
            SOURCE,
            SourceAdapter(ConstantSource {
                channels: 1,
                value: 0.25,
            }),
        ),
        ProcessorInstance::new(GAIN, Gain),
        ProcessorInstance::new(METER, Observer(meter)),
        ProcessorInstance::new(
            SINK,
            SinkAdapter(Capture {
                samples: Arc::clone(&samples),
            }),
        ),
    ])
    .unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }; 2],
        4096,
    )
    .unwrap();
    let src = arena.slot(0).unwrap();
    let dst = if in_place {
        src
    } else {
        arena.slot(1).unwrap()
    };
    let source = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot: src }])
        .unwrap();
    let observer = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot: src }])
        .unwrap();
    let gain = arena
        .prepare_io(&if in_place {
            vec![PortAccess::InPlace {
                input: 0,
                output: 0,
                slot: src,
            }]
        } else {
            vec![
                PortAccess::Read { port: 0, slot: src },
                PortAccess::Write { port: 0, slot: dst },
            ]
        })
        .unwrap();
    let sink = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot: dst }])
        .unwrap();
    // Observer is an additional consumer BEFORE the last-use in-place mutation.
    let plan = ExecutionPlan::prepare(
        arena,
        vec![
            OpSpec {
                processor: SOURCE,
                io: source,
            },
            OpSpec {
                processor: METER,
                io: observer,
            },
            OpSpec {
                processor: GAIN,
                io: gain,
            },
            OpSpec {
                processor: SINK,
                io: sink,
            },
        ],
        &resources,
        &params,
        config(events),
    )
    .unwrap();
    (
        Engine::new(plan, resources, params).unwrap(),
        control,
        reader,
        samples,
    )
}
fn values(samples: &[AtomicU64], frames: usize) -> Vec<f64> {
    samples[..frames]
        .iter()
        .map(|v| f64::from_bits(v.load(Ordering::Relaxed)))
        .collect()
}

#[test]
fn event_at_end_waits_until_next_block_and_ramps_cross_blocks() {
    for in_place in [false, true] {
        let (mut engine, mut control, mut meter, samples) = pipeline(in_place, 16, 16);
        assert_eq!(
            control
                .submit(request(1, ApplyAt::Frame(3), 2.0, 2), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(
            control
                .submit(request(2, ApplyAt::Frame(8), 0.0, 0), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(engine.render(4).unwrap().segments, 2);
        engine.render(4).unwrap();
        assert_eq!(
            values(&samples, 8),
            vec![0.25, 0.25, 0.25, 0.375, 0.5, 0.5, 0.5, 0.5]
        );
        assert_eq!(control.poll_applied().unwrap().effective_frame, 3);
        assert!(control.poll_applied().is_none());
        engine.render(1).unwrap();
        assert_eq!(values(&samples, 9)[8], 0.0);
        assert_eq!(control.poll_applied().unwrap().effective_frame, 8);
        let pre_gain = meter.latest().unwrap();
        assert_eq!(pre_gain.peak, 0.25);
        assert_eq!(pre_gain.rms, 0.25);
    }
}

#[test]
fn ingress_rejects_stale_bad_typed_unordered_and_full_requests() {
    let (mut engine, mut control, _, _) = pipeline(true, 2, 2);
    let base = request(1, ApplyAt::Frame(4), 1.0, 0);
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    plan_revision: REV + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleRevision
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    timeline_epoch: EPOCH + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleEpoch
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Bool(true),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Float(f64::NAN),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(request(1, ApplyAt::Frame(2048), 1.0, 0), 0)
            .code,
        ReplyCode::InvalidTime
    );
    assert_eq!(control.submit(base, 0).code, ReplyCode::Accepted);
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(3), 1.0, 0), 0)
            .code,
        ReplyCode::OutOfOrder
    );
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 2.0, 0), 0)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(
        control
            .submit(request(3, ApplyAt::Frame(4), 3.0, 0), 0)
            .code,
        ReplyCode::QueueFull
    );
    engine.render(8).unwrap();
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    assert_eq!(control.poll_applied().unwrap().request_id, 2);
}

#[test]
fn reply_backpressure_never_stalls_audio_or_loses_an_applied_ack() {
    let (mut engine, mut control, _, samples) = pipeline(true, 4, 1);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    engine.render(4).unwrap();
    // Applied reply is still queued. A second command is accepted but deferred.
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 0.0, 0), 4)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(engine.render(4).unwrap().applied_events, 0);
    assert_eq!(engine.timeline(), 8);
    assert_eq!(values(&samples, 8), vec![0.5; 8]);
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    engine.render(1).unwrap();
    let ack = control.poll_applied().unwrap();
    assert_eq!(ack.request_id, 2);
    assert_eq!(ack.code, ReplyCode::AppliedLate);
    assert_eq!(ack.effective_frame, 8);
    assert_eq!(values(&samples, 9)[8], 0.0);
}

#[test]
fn work_budget_bounds_segmentation_and_defers_excess_events() {
    let (mut engine, mut control, _, _) = pipeline(true, 1, 8);
    for frame in 1..=3 {
        control.submit(request(frame, ApplyAt::Frame(frame), frame as f64, 0), 0);
    }
    let report = engine.render(8).unwrap();
    assert_eq!(report.applied_events, 1);
    assert_eq!(report.segments, 2);
    engine.render(8).unwrap();
    control.poll_applied().unwrap();
    let late = control.poll_applied().unwrap();
    assert_eq!(late.code, ReplyCode::AppliedLate);
    assert_eq!(late.effective_frame, 8);
}

#[test]
fn builtins_render_without_allocation_or_deallocation_even_with_full_telemetry() {
    let (mut engine, mut control, mut meter, _) = pipeline(true, 16, 16);
    for frame in 1..=8 {
        control.submit(request(frame, ApplyAt::Frame(frame), 2.0, 16), 0);
    }
    COUNTS.with(|n| n.set((0, 0)));
    TRACK.with(|t| t.set(true));
    let result = engine.render(16);
    TRACK.with(|t| t.set(false));
    result.unwrap();
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert!(meter.latest().is_some());
    engine.render(16).unwrap();
    assert!(meter.latest().unwrap().dropped_snapshots > 0);
}

#[test]
fn prepared_tables_cannot_be_replaced_by_same_shaped_foreign_tables() {
    let (_, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let (_, foreign) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let resources = RtResources::<f64>::new(vec![ProcessorInstance::new(GAIN, Gain)]).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec {
            processor: GAIN,
            io,
        }],
        &resources,
        &params,
        config(8),
    )
    .unwrap();
    assert!(matches!(
        Engine::new(plan, resources, foreign),
        Err(RuntimeError::ForeignParameters)
    ));
}

#[test]
fn decoding_happens_before_spsc_and_applied_reply_roundtrips() {
    let (mut engine, mut control, _, _) = pipeline(false, 8, 8);
    let mut wire = Vec::new();
    write_parameter(&mut wire, request(10, ApplyAt::NextBlock, 0.5, 0)).unwrap();
    let decoded = read_parameter(&mut wire.as_slice()).unwrap();
    let accepted = control.submit(decoded, engine.timeline());
    assert_eq!(accepted.code, ReplyCode::Accepted);
    engine.render(1).unwrap();
    let applied = control.poll_applied().unwrap();
    let mut response = Vec::new();
    write_reply(&mut response, applied).unwrap();
    assert_eq!(
        read_reply(&mut response.as_slice()).unwrap().code,
        ReplyCode::Applied
    );
}

#[test]
fn engine_and_queue_endpoints_move_to_distinct_threads_without_shared_arena() {
    let (engine, mut control, _, samples) = pipeline(true, 8, 8);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut engine = engine;
        engine.render(8).unwrap();
        // Test synchronization occurs OUTSIDE render, not in a processor.
        ready_tx.send(()).unwrap();
        continue_rx.recv().unwrap();
        engine.render(8).unwrap();
        engine
    });
    ready_rx.recv().unwrap();
    assert_eq!(control.poll_applied().unwrap().effective_frame, 0);
    control.submit(request(2, ApplyAt::Frame(8), 0.0, 0), 8);
    continue_tx.send(()).unwrap();
    let engine = join.join().unwrap();
    assert_eq!(engine.timeline(), 16);
    assert_eq!(values(&samples, 16), [vec![0.5; 8], vec![0.0; 8]].concat());
    // Owned resources return to the non-RT owner before destruction.
    drop(engine.into_parts());
}
