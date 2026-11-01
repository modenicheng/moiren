//! An offline source -> pre-gain meter -> in-place gain -> post-gain meter graph.
//! No audio device is opened and no audible signal is played. The meters are
//! demand-gated signal observers: levels flow through the signal bus, so a
//! subscriber must exist before rendering for any window to be published.
use anyhow::Context;
use moiren_core::protocol::*;
use moiren_engine::{
    boundary::*,
    buffer::*,
    control::*,
    meter::signal::{AudioWindow, ChannelLevel, SignalLevelMeter},
    processor::*,
    runtime::*,
};
use moiren_signal::{BlockInit, Bus, LatestBlock, PumpBudget, Topic};

fn register_level_topic(
    bus: &mut Bus,
    name: &str,
) -> anyhow::Result<Topic<LatestBlock<AudioWindow, ChannelLevel>>> {
    bus.register_latest_block(
        name,
        BlockInit {
            header: AudioWindow {
                timeline_epoch: 0,
                frame_start: 0,
                frame_end: 0,
                sample_rate: 0.0,
            },
            element: ChannelLevel {
                peak: 0.0,
                rms: 0.0,
            },
            capacity: 2,
        },
    )
    .with_context(|| format!("registering the {name} topic"))
}

fn main() -> anyhow::Result<()> {
    let source_id = ProcessorId(1);
    let gain_id = ProcessorId(2);
    let pre_id = ProcessorId(3);
    let post_id = ProcessorId(4);
    let (mut control, parameters) =
        parameter_channel(vec![Gain::parameter(gain_id, 1.0)], 1, 1, 16, 48000)
            .context("creating the parameter channel")?;
    let mut bus = Bus::new(Default::default()).context("creating the signal bus")?;
    let pre_topic = register_level_topic(&mut bus, "offline/pre-gain")?;
    let post_topic = register_level_topic(&mut bus, "offline/post-gain")?;
    let (signal_control, rt_host) = bus.create_host(2).context("creating the signal host")?;
    let key = signal_control
        .new_binding()
        .context("reserving a signal binding key")?;
    let pre_publisher = signal_control
        .prepare(&pre_topic, key)
        .context("preparing the pre-gain level publisher")?;
    let post_publisher = signal_control
        .prepare(&post_topic, key)
        .context("preparing the post-gain level publisher")?;
    let pre = SignalLevelMeter::new(2, 64, pre_publisher).context("creating the pre-gain meter")?;
    let post =
        SignalLevelMeter::new(2, 64, post_publisher).context("creating the post-gain meter")?;
    // Measurement is demand-gated: subscribe before rendering so windows are
    // actually accumulated and published.
    let mut pre_reader = bus
        .subscribe(&pre_topic, Default::default())
        .context("subscribing to the pre-gain topic")?;
    let mut post_reader = bus
        .subscribe(&post_topic, Default::default())
        .context("subscribing to the post-gain topic")?;
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
    let plan = ExecutionPlan::prepare_with_signals(
        arena,
        specs,
        &resources,
        &parameters,
        EngineConfig {
            processing_sr: 48000.0,
            max_block_frames: 256,
            max_events_per_block: 16,
        },
        &signal_control,
    )
    .context("preparing the execution plan")?;
    let mut engine =
        Engine::new(plan, resources, parameters).context("assembling the offline engine")?;
    // The failure owns the host back; only its reason is reportable here.
    engine.attach_signal_host(rt_host).map_err(|failure| {
        anyhow::anyhow!("attaching the signal host failed: {:?}", failure.error)
    })?;
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
    // A sparse directory needs enough topic checks for one full sweep.
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 64,
        copy_bytes: 1 << 20,
    });
    println!(
        "pre: {:?}",
        pre_reader
            .read_latest()
            .map(|block| block.elements().to_vec())
    );
    println!(
        "post: {:?}",
        post_reader
            .read_latest()
            .map(|block| block.elements().to_vec())
    );
    drop(engine.into_parts());
    Ok(())
}
