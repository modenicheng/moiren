use super::{
    allocation::track_allocations,
    support::{EPOCH, GAIN, REV, SINK, SOURCE, request},
};
use moiren_core::protocol::{ApplyAt, ProcessorId, ReplyCode};
use moiren_engine::{
    boundary::audio_bridge,
    buffer::{BufferArena, BufferSlotLayout, PortAccess},
    control::parameter_channel,
    node::{
        DeviceOptions, InputConfig, InputNode, InputSource, OutputConfig, OutputNode, OutputTarget,
    },
    processor::Gain,
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
};

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
    let ((filled, renders, drained_a, drained_b), counts) = track_allocations(|| {
        let filled =
            input.write_interleaved(&[1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
        let renders = [engine.render(4), engine.render(3), engine.render(1)];
        let drained_a = output_a.read_interleaved(&mut a);
        let drained_b = output_b.read_interleaved(&mut b);
        (filled, renders, drained_a, drained_b)
    });
    assert_eq!(counts, (0, 0));
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
