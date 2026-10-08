//! An offline source -> pre-gain meter -> in-place gain -> post-gain meter graph.
//! No audio device is opened and no audible signal is played.
use anyhow::Context;
use moiren_core::protocol::*;
use moiren_engine::{boundary::*, buffer::*, control::*, meter::*, processor::*, runtime::*};

fn main() -> anyhow::Result<()> {
    let source_id = ProcessorId(1);
    let gain_id = ProcessorId(2);
    let pre_id = ProcessorId(3);
    let post_id = ProcessorId(4);
    let (mut control, parameters) =
        parameter_channel(vec![Gain::parameter(gain_id, 1.0)], 1, 1, 16, 48000)
            .context("creating the parameter channel")?;
    let (pre, mut pre_reader) = level_meter(64, 4).context("creating the pre-gain meter")?;
    let (post, mut post_reader) = level_meter(64, 4).context("creating the post-gain meter")?;
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
    .context("assembling processor resources")?;
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 256,
        }],
        8192,
    )
    .context("allocating the buffer arena")?;
    let slot = arena.slot(0).context("reserving the audio slot")?;
    let specs = vec![
        OpSpec {
            processor: source_id,
            io: arena
                .prepare_io(&[PortAccess::Write { port: 0, slot }])
                .context("binding the source output")?,
        },
        OpSpec {
            processor: pre_id,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .context("binding the pre-gain observer")?,
        },
        OpSpec {
            processor: gain_id,
            io: arena
                .prepare_io(&[PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot,
                }])
                .context("binding the in-place gain")?,
        },
        OpSpec {
            processor: post_id,
            io: arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .context("binding the post-gain observer")?,
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
    .context("preparing the execution plan")?;
    let mut engine =
        Engine::new(plan, resources, parameters).context("assembling the offline engine")?;
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
    write_parameter(&mut wire, request).context("encoding the parameter request")?;
    let decoded = read_parameter(&mut wire.as_slice()).context("decoding the parameter request")?;
    let accepted = control.submit(decoded, engine.timeline());
    println!("control: {accepted:?}");
    let report = engine.render(128).context("rendering one block")?;
    println!("render: {report:?}");
    println!("applied: {:?}", control.poll_applied());
    println!("pre: {:?}", pre_reader.latest());
    println!("post: {:?}", post_reader.latest());
    drop(engine.into_parts());
    Ok(())
}
