//! Offline compiler edit -> prepared candidate -> block-boundary swap -> retire.
use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::{ConstantSource, audio_bridge},
    compiler::{CompileConfig, NodeBindings, compile},
    processor::{CompressorSettings, Gain},
    runtime::{EngineConfig, PreparedPlan, ProcessorReuse, RetireOutcome},
};

fn config(revision: u64) -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 8,
        },
        audio_byte_budget: 4096,
        plan_revision: revision,
        timeline_epoch: 1,
        control_capacity: 8,
        control_horizon_frames: 48_000,
    }
}
fn connect(graph: &mut LogicalGraph, from: NodeId, to: NodeId) -> Result<EdgeId, GraphError> {
    graph.connect(
        graph.get_node(from)?.outputs()[0].id(),
        graph.get_node(to)?.inputs()[0].id(),
        SendParams::default(),
    )
}

fn main() -> anyhow::Result<()> {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1)?;
    let gain = graph.create_node(NodeKind::Gain, 1)?;
    let sink = graph.create_node(NodeKind::Sink, 1)?;
    let source_gain = connect(&mut graph, source, gain)?;
    connect(&mut graph, gain, sink)?;
    let (writer, mut output) = audio_bridge::<f64>(1, 16, 4096)?;
    let mut io = NodeBindings::new();
    io.bind_source(
        source,
        ConstantSource {
            channels: 1,
            value: 0.75,
        },
    )?;
    io.bind_sink(sink, writer)?;
    let mut active = compile(&graph, io, config(1))?;
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1)?;
    assert_eq!(
        active
            .control
            .submit(
                ParameterRequest {
                    request_id: 1,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    target: ParameterKey {
                        processor: active.bindings.node(gain).unwrap(),
                        parameter: Gain::LEVEL
                    },
                    at: ApplyAt::NextBlock,
                    value: ParamValue::Float(0.5),
                    ramp_frames: 8,
                },
                0
            )
            .code,
        ReplyCode::Accepted
    );
    active.engine.render(4)?;

    // This edit changes the compiler's processor numbering. Logical NodeId is
    // the mapping source; never assume old ProcessorId matches the new plan.
    graph.disconnect(source_gain)?;
    let compressor = graph.create_node(NodeKind::Compressor, 1)?;
    connect(&mut graph, source, compressor)?;
    connect(&mut graph, compressor, gain)?;
    let (placeholder_writer, placeholder_reader) = audio_bridge::<f64>(1, 16, 4096)?;
    let mut io = NodeBindings::new();
    io.bind_source(
        source,
        ConstantSource {
            channels: 1,
            value: 0.75,
        },
    )?;
    io.bind_gain(gain, 1.0)?;
    io.bind_compressor(
        compressor,
        CompressorSettings {
            threshold_db: -12.0,
            ratio: 2.0,
            attack_ms: 0.0,
            release_ms: 0.0,
            knee_db: 0.0,
            ..CompressorSettings::default()
        },
    )?;
    // Reuse the original sink to retain the output bridge. The unused freshly
    // prepared writer returns in the retirement package and dies on control.
    io.bind_sink(sink, placeholder_writer)?;
    let candidate = compile(&graph, io, config(2))?;
    let reuse: Vec<_> = [source, gain, sink]
        .map(|node| ProcessorReuse {
            old: active.bindings.node(node).unwrap(),
            new: candidate.bindings.node(node).unwrap(),
        })
        .into();
    assert_ne!(reuse[1].old, reuse[1].new);
    let candidate = PreparedPlan::new(candidate.engine)?.with_reuse(&basis, &reuse)?;
    plans.publish(candidate).map_err(|failure| failure.reason)?;
    assert_eq!(plans.active_revision(), 1);
    active.engine.render(4)?;
    assert_eq!(plans.active_revision(), 2);
    assert_eq!(active.engine.timeline(), 8);
    let mut samples = [0.0; 8];
    assert_eq!(output.read_interleaved(&mut samples)?.transferred_frames, 8);
    let compressed = (0.75 * 10.0_f64.powf(-12.0 / 20.0)).sqrt();
    for (i, sample) in samples.iter().enumerate() {
        let level = 1.0 - (i + 1) as f64 / 16.0;
        let signal = if i < 4 { 0.75 } else { compressed };
        assert!((sample - signal * level).abs() < 1e-12);
    }
    let mut retired = plans.poll_retired().expect("one acknowledged swap");
    assert_eq!(
        retired.outcome(),
        RetireOutcome::Replaced {
            active_revision: 2,
            frame: 4
        }
    );
    assert_eq!(retired.reject_pending(), 0);
    println!(
        "revision 1 -> 2 at frame 4; timeline: {}",
        active.engine.timeline()
    );
    println!("continuous gain ramp, inserted compressor: {samples:?}");
    println!("retired revision {} on control", retired.revision());
    drop(retired);
    drop(placeholder_reader);
    drop(active.engine.into_parts()); // render stopped; all remaining ownership is control-side
    Ok(())
}
