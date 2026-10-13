use super::allocation::track_allocations;
use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::{AudioReader, BoundaryReport, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{
        CompileConfig, CompiledGraph, NodeBindings, UpdateReuse, compile, compile_update,
        compile_update_with_reuse,
    },
    processor::{Gain, ProcessContext},
    runtime::{EngineConfig, PlanSwapError, PreparedPlan, RetireOutcome},
};

struct Counter(f64);
impl RtAudioSource<f64> for Counter {
    fn channel_count(&self) -> usize {
        1
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, f64>) -> BoundaryReport {
        for value in output.channel_mut(0) {
            *value = self.0;
            self.0 += 1.0;
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..Default::default()
        }
    }
}

fn config(revision: u64) -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 8,
        },
        audio_byte_budget: 4096,
        plan_revision: revision,
        timeline_epoch: 17,
        control_capacity: 8,
        control_horizon_frames: 1024,
    }
}

fn connect(graph: &mut LogicalGraph, source: NodeId, sink: NodeId) -> EdgeId {
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap()
}

fn initial() -> (
    LogicalGraph,
    [NodeId; 3],
    CompiledGraph<f64>,
    AudioReader<f64>,
) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, gain);
    connect(&mut graph, gain, sink);
    let (writer, reader) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, Counter(1.0)).unwrap();
    bindings.bind_gain(gain, 1.0).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let compiled = compile(&graph, bindings, config(1)).unwrap();
    (graph, [source, gain, sink], compiled, reader)
}

fn samples(reader: &mut AudioReader<f64>, expected: &[f64]) {
    let mut actual = vec![0.0; expected.len()];
    assert_eq!(
        reader
            .read_interleaved(&mut actual)
            .unwrap()
            .transferred_frames,
        expected.len()
    );
    assert_eq!(actual, expected);
}

#[test]
fn update_reserves_live_io_and_preserves_source_sink_and_ramp_without_rt_heap_work() {
    let (mut graph, [source, gain, sink], mut active, mut output) = initial();
    let basis = active.engine.plan_snapshot();
    let old_gain = active.bindings.node(gain).unwrap();
    let mut plans = active.engine.enable_plan_switching(2).unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(sink).unwrap())
        .unwrap();
    active.engine.render(2).unwrap();
    samples(&mut output, &[1.0, 2.0]);
    let reply = active.control.submit(
        ParameterRequest {
            request_id: 1,
            plan_revision: 1,
            timeline_epoch: 17,
            target: ParameterKey {
                processor: old_gain,
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::NextBlock,
            value: ParamValue::Float(0.0),
            ramp_frames: 4,
        },
        2,
    );
    assert_eq!(reply.code, ReplyCode::Accepted);
    active.engine.render(2).unwrap();
    samples(&mut output, &[2.25, 2.0]);
    let edge = graph
        .edges()
        .iter()
        .find(|edge| edge.dst() == sink)
        .unwrap()
        .id();
    graph.disconnect(edge).unwrap();
    let added = graph.create_node(NodeKind::Gain, 1).unwrap();
    connect(&mut graph, gain, added);
    connect(&mut graph, added, sink);
    let mut bindings = NodeBindings::new();
    bindings.bind_gain(gain, 0.25).unwrap();
    bindings.bind_gain(added, 0.5).unwrap();
    let reuse: Vec<_> = [source, gain, sink]
        .into_iter()
        .map(|node| (node, active.bindings.node(node).unwrap()))
        .collect();
    let update = compile_update(&graph, bindings, config(2), &basis, &reuse, None).unwrap();
    assert_ne!(update.bindings.node(sink), active.bindings.node(sink));
    plans.publish(update.plan).unwrap();
    let (rendered, allocations) = track_allocations(|| active.engine.render(2));
    assert_eq!(allocations, (0, 0));
    assert_eq!(rendered.unwrap().end, 6);
    samples(&mut output, &[0.625, 0.0]);
    assert_eq!(plans.active_revision(), 2);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Replaced {
            active_revision: 2,
            frame: 4
        }
    );
    let next_basis = active.engine.plan_snapshot();
    let next_reuse: Vec<_> = [source, gain, added, sink]
        .into_iter()
        .map(|node| (node, update.bindings.node(node).unwrap()))
        .collect();
    let mut next_bindings = NodeBindings::new();
    next_bindings.bind_gain(gain, 1.0).unwrap();
    next_bindings.bind_gain(added, 0.5).unwrap();
    let next = compile_update(
        &graph,
        next_bindings,
        config(3),
        &next_basis,
        &next_reuse,
        None,
    )
    .unwrap();
    plans.publish(next.plan).unwrap();
    active.engine.render(2).unwrap();
    assert_eq!(plans.active_revision(), 3);
    samples(&mut output, &[0.0, 0.0]);
}

#[test]
fn protected_sink_rejects_an_arbitrary_replacement_output_bridge() {
    let (graph, [source, gain, sink], mut active, mut output) = initial();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(sink).unwrap())
        .unwrap();
    let (writer, mut foreign_output) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, Counter(100.0)).unwrap();
    bindings.bind_gain(gain, 0.5).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let candidate = compile(&graph, bindings, config(2)).unwrap();
    plans
        .publish(PreparedPlan::new(candidate.engine).unwrap())
        .unwrap();
    let (_, allocations) = track_allocations(|| active.engine.render(2).unwrap());
    assert_eq!(allocations, (0, 0));
    assert_eq!(plans.active_revision(), 1);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingReuse)
    );
    samples(&mut output, &[1.0, 2.0]);
    let mut unused = [0.0; 2];
    assert_eq!(
        foreign_output
            .read_interleaved(&mut unused)
            .unwrap()
            .transferred_frames,
        0
    );
}

#[test]
fn reserved_io_cannot_clear_its_required_reuse_mapping() {
    let (graph, [source, gain, sink], active, _) = initial();
    let basis = active.engine.plan_snapshot();
    let mut bindings = NodeBindings::<f64>::new();
    bindings.bind_gain(gain, 1.0).unwrap();
    let reuse = [
        (source, active.bindings.node(source).unwrap()),
        (sink, active.bindings.node(sink).unwrap()),
    ];
    let update = compile_update(&graph, bindings, config(2), &basis, &reuse, None).unwrap();
    assert!(matches!(
        update.plan.with_reuse(&basis, &[]),
        Err(PlanSwapError::MissingReuse)
    ));
}

#[test]
fn unrelated_topology_update_preserves_an_active_send_ramp() {
    let (mut graph, [source, gain, sink], mut active, mut output) = initial();
    let edge = graph
        .edges()
        .iter()
        .find(|edge| edge.src() == source)
        .unwrap()
        .id();
    let target = active.bindings.edge(edge).unwrap().gain;
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(sink).unwrap())
        .unwrap();
    assert_eq!(
        active
            .control
            .submit(
                ParameterRequest {
                    request_id: 1,
                    plan_revision: 1,
                    timeline_epoch: 17,
                    target,
                    at: ApplyAt::NextBlock,
                    value: ParamValue::Float(0.5),
                    ramp_frames: 4,
                },
                0
            )
            .code,
        ReplyCode::Accepted
    );
    active.engine.render(2).unwrap();
    samples(&mut output, &[0.875, 1.5]);
    let removed = graph
        .edges()
        .iter()
        .find(|edge| edge.dst() == sink)
        .unwrap()
        .id();
    graph.disconnect(removed).unwrap();
    let added = graph.create_node(NodeKind::Gain, 1).unwrap();
    connect(&mut graph, gain, added);
    connect(&mut graph, added, sink);
    let mut bindings = NodeBindings::new();
    bindings.bind_gain(gain, 1.0).unwrap();
    bindings.bind_gain(added, 1.0).unwrap();
    let reuse: Vec<_> = [source, gain, sink]
        .into_iter()
        .map(|node| (node, active.bindings.node(node).unwrap()))
        .collect();
    let update = compile_update_with_reuse(
        &graph,
        bindings,
        config(2),
        &basis,
        UpdateReuse {
            nodes: &reuse,
            edges: &[(edge, target.processor)],
        },
        None,
    )
    .unwrap();
    plans.publish(update.plan).unwrap();
    let (_, allocations) = track_allocations(|| active.engine.render(2).unwrap());
    assert_eq!(allocations, (0, 0));
    samples(&mut output, &[1.875, 2.0]);
}
