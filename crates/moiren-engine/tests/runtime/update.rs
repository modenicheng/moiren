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

#[test]
fn stopped_owner_returns_pending_candidate_and_terminalizes_accepted_controls() {
    let (graph, [source, gain, sink], mut active, _) = initial();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (writer, _) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, Counter(10.0)).unwrap();
    bindings.bind_gain(gain, 1.0).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut candidate = compile(&graph, bindings, config(2)).unwrap();
    let request = ParameterRequest {
        request_id: 9,
        plan_revision: 2,
        timeline_epoch: 17,
        target: ParameterKey {
            processor: candidate.bindings.node(gain).unwrap(),
            parameter: Gain::LEVEL,
        },
        at: ApplyAt::Frame(100),
        value: ParamValue::Float(0.5),
        ramp_frames: 0,
    };
    assert_eq!(
        candidate.control.submit(request, 0).code,
        ReplyCode::Accepted
    );
    plans
        .publish(PreparedPlan::new(candidate.engine).unwrap())
        .unwrap();
    let mut pending = active.engine.take_pending_plan().unwrap();
    assert_eq!(pending.retire_controls(), 0);
    let reply = candidate.control.poll_applied().unwrap();
    assert_eq!(
        (reply.request_id, reply.code),
        (9, ReplyCode::StaleRevision)
    );
    assert!(active.engine.take_pending_plan().is_none());
    assert_eq!(
        candidate.control.submit(request, 0).code,
        ReplyCode::StaleRevision
    );
}

fn prepared_update_for_existing_nodes(
    graph: &LogicalGraph,
    nodes: [NodeId; 3],
    active: &CompiledGraph<f64>,
    basis: &moiren_engine::runtime::PlanSnapshot,
    revision: u64,
) -> moiren_engine::compiler::CompiledUpdate<f64> {
    let mut bindings = NodeBindings::new();
    bindings.bind_gain(nodes[1], 0.25).unwrap();
    let reuse: Vec<_> = nodes
        .into_iter()
        .map(|node| (node, active.bindings.node(node).unwrap()))
        .collect();
    compile_update(graph, bindings, config(revision), basis, &reuse, None).unwrap()
}

fn future_gain_request(target: ParameterKey) -> ParameterRequest {
    ParameterRequest {
        request_id: 9001,
        plan_revision: 2,
        timeline_epoch: 17,
        target,
        at: ApplyAt::Frame(100),
        value: ParamValue::Float(0.5),
        ramp_frames: 0,
    }
}

#[test]
fn retired_candidate_cannot_be_published() {
    let (graph, nodes, mut active, _) = initial();
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let mut update = prepared_update_for_existing_nodes(&graph, nodes, &active, &basis, 2);
    assert_eq!(update.plan.retire_controls(), 0);
    match plans.publish(update.plan) {
        Err(failure) => assert_eq!(failure.reason, PlanSwapError::InvalidCandidate),
        Ok(()) => {
            active.engine.render(1).unwrap();
            assert_eq!(plans.active_revision(), 2);
            let request = future_gain_request(ParameterKey {
                processor: update.bindings.node(nodes[1]).unwrap(),
                parameter: Gain::LEVEL,
            });
            assert_eq!(
                update.control.submit(request, 1).code,
                ReplyCode::StaleRevision
            );
            panic!(
                "retired candidate activated at revision 2 with a permanently retired control channel"
            );
        }
    }
}

#[test]
fn failed_reuse_returns_terminal_reply_for_accepted_candidate_request() {
    let (graph, nodes, active, _) = initial();
    let basis = active.engine.plan_snapshot();
    let mut update = prepared_update_for_existing_nodes(&graph, nodes, &active, &basis, 2);
    let request = future_gain_request(ParameterKey {
        processor: update.bindings.node(nodes[1]).unwrap(),
        parameter: Gain::LEVEL,
    });
    assert_eq!(update.control.submit(request, 0).code, ReplyCode::Accepted);
    assert!(matches!(
        update.plan.with_reuse(&basis, &[]),
        Err(PlanSwapError::MissingReuse)
    ));
    assert_eq!(
        update.control.submit(request, 0).code,
        ReplyCode::StaleRevision
    );
    assert!(
        update.control.poll_applied().is_some(),
        "accepted request 9001 disappeared on with_reuse(MissingReuse); the failure returns no candidate to retire"
    );
}

#[test]
fn protected_sink_rejects_foreign_output_after_slot_migration() {
    use moiren_engine::runtime::ProcessorReuse;
    let (mut graph, [source, gain, sink], mut active, mut output) = initial();
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(2).unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(sink).unwrap())
        .unwrap();
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
    bindings.bind_gain(gain, 1.0).unwrap();
    bindings.bind_gain(added, 1.0).unwrap();
    let reuse: Vec<_> = [source, gain, sink]
        .into_iter()
        .map(|node| (node, active.bindings.node(node).unwrap()))
        .collect();
    let update = compile_update(&graph, bindings, config(2), &basis, &reuse, None).unwrap();
    assert_eq!(active.bindings.node(sink), update.bindings.node(added));
    assert_ne!(active.bindings.node(sink), update.bindings.node(sink));
    plans.publish(update.plan).unwrap();
    active.engine.render(1).unwrap();
    drop(plans.poll_retired().unwrap());
    samples(&mut output, &[1.0]);
    let next_basis = active.engine.plan_snapshot();
    let (writer, mut foreign_output) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, Counter(100.0)).unwrap();
    bindings.bind_gain(gain, 1.0).unwrap();
    bindings.bind_gain(added, 1.0).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let foreign = compile(&graph, bindings, config(3)).unwrap();
    let candidate = PreparedPlan::new(foreign.engine)
        .unwrap()
        .with_reuse(
            &next_basis,
            &[ProcessorReuse {
                old: update.bindings.node(added).unwrap(),
                new: foreign.bindings.node(added).unwrap(),
            }],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    let (rendered, heap) = track_allocations(|| active.engine.render(1));
    rendered.unwrap();
    assert_eq!(heap, (0, 0));
    assert_eq!(plans.active_revision(), 2);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingReuse)
    );
    samples(&mut output, &[2.0]);
    let mut unused = [0.0];
    assert_eq!(
        foreign_output
            .read_interleaved(&mut unused)
            .unwrap()
            .transferred_frames,
        0
    );
}

#[test]
fn reserved_input_reader_survives_owner_thread_swap_stale_basis_and_cancellation() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, sink);
    let (mut input, reader) = audio_bridge::<f64>(1, 32, 4096).unwrap();
    let (writer, mut output) = audio_bridge::<f64>(1, 32, 4096).unwrap();
    input
        .write_interleaved(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])
        .unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, reader).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let compiled = compile(&graph, bindings, config(1)).unwrap();
    let mut engine = compiled.engine;
    let basis = engine.plan_snapshot();
    let mut plans = engine.enable_plan_switching(2).unwrap();
    engine
        .require_sink_reuse(compiled.bindings.node(sink).unwrap())
        .unwrap();
    let (commands, command_reader) = std::sync::mpsc::channel();
    let (reports, report_reader) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        while command_reader.recv().unwrap() {
            let (rendered, heap) = super::allocation::track_heap(|| engine.render(2));
            reports
                .send((rendered.unwrap(), heap, engine.plan_snapshot()))
                .unwrap();
        }
        engine
    });
    commands.send(true).unwrap();
    assert_eq!(report_reader.recv().unwrap().1, (0, 0, 0));
    samples(&mut output, &[1.0, 2.0]);
    let reuse = [
        (source, compiled.bindings.node(source).unwrap()),
        (sink, compiled.bindings.node(sink).unwrap()),
    ];
    let update =
        compile_update(&graph, NodeBindings::new(), config(2), &basis, &reuse, None).unwrap();
    let stale =
        compile_update(&graph, NodeBindings::new(), config(3), &basis, &reuse, None).unwrap();
    plans.publish(update.plan).unwrap();
    commands.send(true).unwrap();
    let (_, heap, next_basis) = report_reader.recv().unwrap();
    assert_eq!(heap, (0, 0, 0));
    samples(&mut output, &[3.0, 4.0]);
    drop(plans.poll_retired().unwrap());
    plans.publish(stale.plan).unwrap();
    commands.send(true).unwrap();
    assert_eq!(report_reader.recv().unwrap().1, (0, 0, 0));
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::StaleBasis)
    );
    samples(&mut output, &[5.0, 6.0]);
    let next_reuse = [
        (source, update.bindings.node(source).unwrap()),
        (sink, update.bindings.node(sink).unwrap()),
    ];
    let cancelled = compile_update(
        &graph,
        NodeBindings::new(),
        config(4),
        &next_basis,
        &next_reuse,
        None,
    )
    .unwrap();
    plans.publish(cancelled.plan).unwrap();
    plans.cancel_pending();
    commands.send(true).unwrap();
    assert_eq!(report_reader.recv().unwrap().1, (0, 0, 0));
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::Cancelled)
    );
    samples(&mut output, &[7.0, 8.0]);
    commands.send(false).unwrap();
    let engine = worker.join().unwrap();
    assert_eq!(engine.revision(), 2);
    assert_eq!(engine.timeline(), 8);
}

#[test]
fn retired_candidate_cannot_replace_reuse_mappings() {
    let (graph, nodes, active, _) = initial();
    let basis = active.engine.plan_snapshot();
    let mut update = prepared_update_for_existing_nodes(&graph, nodes, &active, &basis, 2);
    assert_eq!(update.plan.retire_controls(), 0);
    assert!(matches!(
        update.plan.with_reuse(&basis, &[]),
        Err(PlanSwapError::InvalidCandidate)
    ));
}

#[test]
fn every_protected_sink_survives_partial_reuse_rejection_and_slot_migration_without_rt_heap_work() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let a = graph.create_node(NodeKind::Sink, 1).unwrap();
    let b = graph.create_node(NodeKind::Sink, 1).unwrap();
    let edge_a = connect(&mut graph, source, a);
    connect(&mut graph, source, b);
    let (writer_a, mut output_a) = audio_bridge(1, 32, 4096).unwrap();
    let (writer_b, mut output_b) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, Counter(1.0)).unwrap();
    bindings.bind_sink(a, writer_a).unwrap();
    bindings.bind_sink(b, writer_b).unwrap();
    let mut active = compile(&graph, bindings, config(1)).unwrap();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(a).unwrap())
        .unwrap();
    active
        .engine
        .require_sink_reuse(active.bindings.node(b).unwrap())
        .unwrap();
    let basis = active.engine.plan_snapshot();
    let (foreign, mut foreign_output) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_sink(a, foreign).unwrap();
    let partial = compile_update(
        &graph,
        bindings,
        config(2),
        &basis,
        &[
            (source, active.bindings.node(source).unwrap()),
            (b, active.bindings.node(b).unwrap()),
        ],
        None,
    )
    .unwrap();
    plans.publish(partial.plan).unwrap();
    let (_, allocations) = track_allocations(|| active.engine.render(2).unwrap());
    assert_eq!(allocations, (0, 0));
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingReuse)
    );
    samples(&mut output_a, &[1.0, 2.0]);
    samples(&mut output_b, &[1.0, 2.0]);
    assert_eq!(
        foreign_output
            .read_interleaved(&mut [0.0; 2])
            .unwrap()
            .transferred_frames,
        0
    );
    graph.disconnect(edge_a).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 1).unwrap();
    connect(&mut graph, source, gain);
    connect(&mut graph, gain, a);
    let mut bindings = NodeBindings::new();
    bindings.bind_gain(gain, 0.5).unwrap();
    let reuse: Vec<_> = [source, a, b]
        .map(|node| (node, active.bindings.node(node).unwrap()))
        .into();
    let update = compile_update(&graph, bindings, config(3), &basis, &reuse, None).unwrap();
    plans.publish(update.plan).unwrap();
    let (_, allocations) = track_allocations(|| active.engine.render(2).unwrap());
    assert_eq!(allocations, (0, 0));
    assert!(matches!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Replaced {
            active_revision: 3,
            ..
        }
    ));
    samples(&mut output_a, &[1.5, 2.0]);
    samples(&mut output_b, &[3.0, 4.0]);
    let (foreign, _) = audio_bridge(1, 32, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_sink(b, foreign).unwrap();
    bindings.bind_gain(gain, 0.5).unwrap();
    let reuse: Vec<_> = [source, gain, a]
        .map(|node| (node, update.bindings.node(node).unwrap()))
        .into();
    let partial = compile_update(
        &graph,
        bindings,
        config(4),
        &active.engine.plan_snapshot(),
        &reuse,
        None,
    )
    .unwrap();
    plans.publish(partial.plan).unwrap();
    let (_, allocations) = track_allocations(|| active.engine.render(2).unwrap());
    assert_eq!(allocations, (0, 0));
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingReuse)
    );
    samples(&mut output_a, &[2.5, 3.0]);
    samples(&mut output_b, &[5.0, 6.0]);
}
