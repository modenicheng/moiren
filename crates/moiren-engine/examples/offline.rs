//! An offline source -> pre-gain meter -> in-place gain -> post-gain meter graph.
//! No audio device is opened and no audible signal is played.
use moiren_core::protocol::*;
use moiren_engine::{boundary::*, buffer::*, control::*, meter::*, processor::*, runtime::*};

fn main() {
    let source_id = ProcessorId(1);
    let gain_id = ProcessorId(2);
    let pre_id = ProcessorId(3);
    let post_id = ProcessorId(4);
    let (mut control, parameters) =
        parameter_channel(vec![Gain::parameter(gain_id, 1.0)], 1, 1, 16, 48000).unwrap();
    let (pre, mut pre_reader) = level_meter(64, 4).unwrap();
    let (post, mut post_reader) = level_meter(64, 4).unwrap();
    let resources = RtResources::<f32>::new(vec![
        ProcessorInstance::new(
            source_id,
            SourceAdapter(ConstantSource {
                channels: 2,
                value: 0.25,
            }),
        ),
        ProcessorInstance::new(pre_id, Observer(pre)),
        ProcessorInstance::new(gain_id, Gain),
        ProcessorInstance::new(post_id, Observer(post)),
    ])
    .unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 256,
        }],
        8192,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let specs = vec![
        OpSpec {
            processor: source_id,
            io: arena
                .prepare_io(&[PortAccess::Write { port: 0, slot }])
                .unwrap(),
        },
        OpSpec {
            processor: pre_id,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap(),
        },
        OpSpec {
            processor: gain_id,
            io: arena
                .prepare_io(&[PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot,
                }])
                .unwrap(),
        },
        OpSpec {
            processor: post_id,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap(),
        },
    ];
    let plan = ExecutionPlan::prepare(
        arena,
        specs,
        &resources,
        &parameters,
        EngineConfig {
            processing_sr: 48000.0,
            max_block_frames: 256,
            max_events_per_block: 16,
        },
    )
    .unwrap();
    let mut engine = Engine::new(plan, resources, parameters).unwrap();
    // The same codec can run in a Named Pipe worker. It never runs inside render.
    let request = ParameterRequest {
        request_id: 1,
        plan_revision: 1,
        timeline_epoch: 1,
        target: ParameterKey {
            processor: gain_id,
            parameter: Gain::LEVEL,
        },
        at: ApplyAt::Frame(32),
        value: ParamValue::Float(0.5),
        ramp_frames: 16,
    };
    let mut wire = Vec::new();
    write_parameter(&mut wire, request).unwrap();
    let accepted = control.submit(
        read_parameter(&mut wire.as_slice()).unwrap(),
        engine.timeline(),
    );
    println!("control: {accepted:?}");
    let report = engine.render(128).unwrap();
    println!("render: {report:?}");
    println!("applied: {:?}", control.poll_applied());
    println!("pre: {:?}", pre_reader.latest());
    println!("post: {:?}", post_reader.latest());
    drop(engine.into_parts());
}
