use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::*, compiler::*, control::ControlError, processor::*, runtime::*,
    sample::ProcessingSample,
};

fn config() -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 16,
        },
        audio_byte_budget: 65_536,
        plan_revision: 7,
        timeline_epoch: 3,
        control_capacity: 16,
        control_horizon_frames: 48_000,
    }
}

fn output(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().outputs()[0].id()
}
fn input(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().inputs()[0].id()
}
fn connect(graph: &mut LogicalGraph, a: NodeId, b: NodeId, params: SendParams) -> EdgeId {
    graph
        .connect(output(graph, a), input(graph, b), params)
        .unwrap()
}
fn constant<S: ProcessingSample>(bindings: &mut NodeBindings<S>, node: NodeId, channels: usize) {
    bindings
        .bind_source(
            node,
            ConstantSource {
                channels,
                value: 0.375,
            },
        )
        .unwrap();
}
fn assert_samples<S: ProcessingSample>(actual: &[S], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual.to_f64() - expected).abs() < 1e-6,
            "{} != {expected}",
            actual.to_f64()
        );
    }
}

fn example_ramp<S: ProcessingSample>() {
    let mut graph = LogicalGraph::new();
    let a = graph.create_node(NodeKind::Source, 2).unwrap();
    let b = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let pan = graph.create_node(NodeKind::Pan, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    for source in [a, b] {
        let port = graph.add_input_port(bus, 2).unwrap();
        graph
            .connect(output(&graph, source), port, SendParams::default())
            .unwrap();
    }
    connect(&mut graph, bus, pan, SendParams::default());
    connect(&mut graph, pan, sink, SendParams::default());
    let (writer, mut reader) = audio_bridge::<S>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            a,
            ConstantSource {
                channels: 2,
                value: 0.25,
            },
        )
        .unwrap();
    bindings
        .bind_source(
            b,
            ConstantSource {
                channels: 2,
                value: 0.125,
            },
        )
        .unwrap();
    bindings.bind_pan(pan, -1.0).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    assert_eq!(compiled.stats.node_count, 5);
    assert_eq!(compiled.stats.edge_count, 4);
    assert_eq!(compiled.stats.operation_count, 9);
    let id = compiled.bindings.node(pan).unwrap();
    let ack = compiled.control.submit(
        ParameterRequest {
            request_id: 1,
            plan_revision: 7,
            timeline_epoch: 3,
            target: ParameterKey {
                processor: id,
                parameter: Pan::POSITION,
            },
            at: ApplyAt::Frame(2),
            value: ParamValue::Float(1.0),
            ramp_frames: 4,
        },
        0,
    );
    assert_eq!(ack.code, ReplyCode::Accepted);
    for frames in [3, 1, 4] {
        compiled.engine.render(frames).unwrap();
    }
    let mut samples = [S::ZERO; 16];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        8
    );
    assert_samples(
        &samples,
        &[
            0.375, 0.0, 0.375, 0.0, 0.375, 0.1875, 0.375, 0.375, 0.1875, 0.375, 0.0, 0.375, 0.0,
            0.375, 0.0, 0.375,
        ],
    );
    assert_eq!(
        compiled.control.poll_applied().unwrap().code,
        ReplyCode::Applied
    );
}

#[test]
fn compiled_example_preserves_cross_block_pan_ramp_in_both_precisions() {
    example_ramp::<f32>();
    example_ramp::<f64>();
}

#[test]
fn sends_preserve_fan_out_and_accept_gain_above_the_node_gain_limit() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let raw = graph.create_node(NodeKind::Sink, 2).unwrap();
    let sent = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, raw, SendParams::default());
    let edge = connect(
        &mut graph,
        source,
        sent,
        SendParams {
            gain: 32.0,
            pan: 0.5,
            ..SendParams::default()
        },
    );
    let (raw_writer, mut raw_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let (send_writer, mut send_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_sink(raw, raw_writer).unwrap();
    bindings.bind_sink(sent, send_writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let keys = compiled.bindings.edge(edge).unwrap();
    for (request_id, target, value) in [
        (1, keys.gain, ParamValue::Float(2.0)),
        (2, keys.pan, ParamValue::Float(-1.0)),
        (3, keys.mute, ParamValue::Bool(true)),
    ] {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id,
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target,
                        at: ApplyAt::Frame(2),
                        value,
                        ramp_frames: 0,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    compiled.engine.render(4).unwrap();
    let mut raw_samples = [0.0; 8];
    let mut sent_samples = [0.0; 8];
    raw_reader.read_interleaved(&mut raw_samples).unwrap();
    send_reader.read_interleaved(&mut sent_samples).unwrap();
    assert_eq!(raw_samples, [0.375; 8]);
    assert_eq!(sent_samples, [6.0, 12.0, 6.0, 12.0, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn repeated_bus_sources_and_disconnected_inputs_are_not_lost() {
    for channels in [1, 2, 4] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let bus = graph.create_node(NodeKind::Bus, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        graph.add_input_port(bus, channels).unwrap(); // retained but unconnected
        for _ in 0..2 {
            let port = graph.add_input_port(bus, channels).unwrap();
            graph
                .connect(output(&graph, source), port, SendParams::default())
                .unwrap();
        }
        connect(&mut graph, bus, sink, SendParams::default());
        let (writer, mut reader) = audio_bridge::<f64>(channels, 16, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, channels);
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        for frames in [8, 1, 3] {
            compiled.engine.render(frames).unwrap();
            let mut samples = vec![0.0; frames * channels];
            assert_eq!(
                reader
                    .read_interleaved(&mut samples)
                    .unwrap()
                    .transferred_frames,
                frames
            );
            assert_eq!(samples, vec![0.75; frames * channels]);
        }
    }
}

#[test]
fn empty_graph_empty_bus_and_unconnected_fixed_inputs_produce_silence() {
    let mut empty = compile::<f64>(&LogicalGraph::new(), NodeBindings::new(), config()).unwrap();
    assert_eq!(empty.stats.slot_count, 0);
    assert_eq!(empty.engine.render(8).unwrap().end, 8);
    for kind in [NodeKind::Bus, NodeKind::Gain, NodeKind::Pan, NodeKind::Sink] {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(kind, 2).unwrap();
        let sink = if kind == NodeKind::Sink {
            node
        } else {
            let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
            connect(&mut graph, node, sink, SendParams::default());
            sink
        };
        let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        for frames in [8, 1, 4] {
            compiled.engine.render(frames).unwrap();
            let mut samples = vec![1.0; frames * 2];
            reader.read_interleaved(&mut samples).unwrap();
            assert_eq!(samples, vec![0.0; frames * 2]);
        }
    }
}

#[test]
fn unsupported_send_semantics_are_rejected_instead_of_ignored() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    let edge = connect(
        &mut graph,
        source,
        sink,
        SendParams {
            tap: SendTap::PreFader,
            ..SendParams::default()
        },
    );
    assert!(
        matches!(compile::<f64>(&graph, NodeBindings::new(), config()), Err(CompileError::UnsupportedTap { edge: id }) if id == edge)
    );
    graph
        .set_send_params(
            edge,
            SendParams {
                pan: 0.5,
                ..SendParams::default()
            },
        )
        .unwrap();
    assert!(
        matches!(compile::<f64>(&graph, NodeBindings::new(), config()), Err(CompileError::UnsupportedPan { edge: id, channels: 1 }) if id == edge)
    );
}

#[test]
fn bindings_and_configuration_fail_before_an_engine_is_published() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    assert!(
        matches!(compile::<f64>(&graph, NodeBindings::new(), config()), Err(CompileError::MissingBinding { node }) if node == source)
    );
    let mut bindings = NodeBindings::<f64>::new();
    constant(&mut bindings, source, 2);
    assert!(
        matches!(bindings.bind_gain(source, 1.0), Err(CompileError::DuplicateBinding { node }) if node == source)
    );
    let mut wrong = NodeBindings::<f64>::new();
    wrong.bind_gain(source, 1.0).unwrap();
    assert!(
        matches!(compile(&graph, wrong, config()), Err(CompileError::BindingKind { node }) if node == source)
    );
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let mut bad_value = NodeBindings::<f64>::new();
    constant(&mut bad_value, source, 2);
    bad_value.bind_gain(gain, f64::NAN).unwrap();
    assert!(matches!(
        compile(&graph, bad_value, config()),
        Err(CompileError::Control(ControlError::InvalidConfiguration))
    ));
    let mut too_small = config();
    too_small.audio_byte_budget = 1;
    let mut bindings = NodeBindings::<f64>::new();
    constant(&mut bindings, source, 2);
    assert!(matches!(
        compile(&graph, bindings, too_small),
        Err(CompileError::Buffer(_))
    ));
    let mut bad_config = config();
    bad_config.engine.max_block_frames = 0;
    assert!(matches!(
        compile::<f64>(&graph, NodeBindings::new(), bad_config),
        Err(CompileError::Runtime(RuntimeError::InvalidConfig))
    ));
    let deleted = graph.create_node(NodeKind::Bus, 2).unwrap();
    graph.remove_node(deleted).unwrap();
    let mut stale = NodeBindings::<f64>::new();
    stale.bind_gain(deleted, 1.0).unwrap();
    assert!(matches!(
        compile(&graph, stale, config()),
        Err(CompileError::Graph(GraphError::NodeNotFound))
    ));
}

fn random_dags<S: ProcessingSample>() {
    let mut rng = 0x73b7_9816_u64;
    let mut next = || {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (rng >> 32) as usize
    };
    for _ in 0..24 {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let mut nodes = vec![source];
        let mut expected = vec![[0.375, 0.375]];
        let mut bindings = NodeBindings::<S>::new();
        constant(&mut bindings, source, 2);
        for _ in 0..10 {
            let kind = [NodeKind::Gain, NodeKind::Pan, NodeKind::Bus][next() % 3];
            let node = graph.create_node(kind, 2).unwrap();
            let mut value = [0.0, 0.0];
            let ports = if kind == NodeKind::Bus {
                (0..next() % 4)
                    .map(|_| graph.add_input_port(node, 2).unwrap())
                    .collect::<Vec<_>>()
            } else {
                vec![input(&graph, node)]
            };
            for port in ports {
                if next() % 5 == 0 {
                    continue;
                }
                let index = next() % nodes.len();
                let params = SendParams {
                    gain: [0.0, 0.5, 1.0, 2.0][next() % 4],
                    pan: [-1.0, -0.5, 0.0, 0.5, 1.0][next() % 5],
                    mute: next() % 7 == 0,
                    ..SendParams::default()
                };
                graph
                    .connect(output(&graph, nodes[index]), port, params)
                    .unwrap();
                if !params.mute {
                    // Independent sample evaluator; no compiler IR/DSP helpers.
                    value[0] += expected[index][0] * params.gain * (1.0 - params.pan.max(0.0));
                    value[1] += expected[index][1] * params.gain * (1.0 + params.pan.min(0.0));
                }
            }
            if kind == NodeKind::Gain {
                let gain = [0.25, 0.5, 1.0, 2.0][next() % 4];
                bindings.bind_gain(node, gain).unwrap();
                value[0] *= gain;
                value[1] *= gain;
            } else if kind == NodeKind::Pan {
                let pan: f64 = [-1.0, -0.5, 0.0, 0.5, 1.0][next() % 5];
                bindings.bind_pan(node, pan).unwrap();
                value[0] *= 1.0 - pan.max(0.0);
                value[1] *= 1.0 + pan.min(0.0);
            }
            nodes.push(node);
            expected.push(value);
        }
        let mut sinks = Vec::new();
        for index in [0, next() % nodes.len(), nodes.len() - 1] {
            let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
            connect(&mut graph, nodes[index], sink, SendParams::default());
            let (writer, reader) = audio_bridge::<S>(2, 16, 4096).unwrap();
            bindings.bind_sink(sink, writer).unwrap();
            sinks.push((reader, expected[index]));
        }
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        for frames in [1, 8, 3, 2] {
            compiled.engine.render(frames).unwrap();
            for (reader, value) in &mut sinks {
                let mut samples = vec![S::ZERO; frames * 2];
                assert_eq!(
                    reader
                        .read_interleaved(&mut samples)
                        .unwrap()
                        .transferred_frames,
                    frames
                );
                let reference = (0..frames).flat_map(|_| *value).collect::<Vec<_>>();
                assert_samples(&samples, &reference);
            }
        }
    }
}

#[test]
fn random_dags_match_independent_sample_evaluation_for_both_precisions() {
    random_dags::<f32>();
    random_dags::<f64>();
}

#[test]
fn topology_edits_are_reflected_by_recompilation_without_changing_the_graph() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let direct = connect(&mut graph, source, sink, SendParams::default());
    for expected in [0.375, 0.1875] {
        let snapshot = graph.clone();
        let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 2);
        bindings.bind_gain(gain, 0.5).unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        assert_eq!(graph, snapshot);
        compiled.engine.render(1).unwrap();
        let mut samples = [0.0; 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples, [expected; 2]);
        if expected == 0.375 {
            graph.disconnect(direct).unwrap();
            connect(&mut graph, source, gain, SendParams::default());
            connect(&mut graph, gain, sink, SendParams::default());
        }
    }
}

#[test]
fn edge_gain_and_pan_ramps_cross_blocks_and_nonstereo_pan_updates_are_rejected() {
    for channels in [1, 2, 4] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        let edge = connect(&mut graph, source, sink, SendParams::default());
        let (writer, mut reader) = audio_bridge::<f64>(channels, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, channels);
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let keys = compiled.bindings.edge(edge).unwrap();
        let request = ParameterRequest {
            request_id: 1,
            plan_revision: 7,
            timeline_epoch: 3,
            target: keys.pan,
            at: ApplyAt::Frame(1),
            value: ParamValue::Float(1.0),
            ramp_frames: 4,
        };
        assert_eq!(
            compiled.control.submit(request, 0).code,
            if channels == 2 {
                ReplyCode::Accepted
            } else {
                ReplyCode::InvalidValue
            }
        );
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: 2,
                        target: keys.gain,
                        value: ParamValue::Float(0.0),
                        ..request
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
        for frames in [2, 1, 3] {
            compiled.engine.render(frames).unwrap();
        }
        let mut samples = vec![0.0; channels * 6];
        reader.read_interleaved(&mut samples).unwrap();
        let expected = (0..6)
            .flat_map(|frame| {
                let progress = if frame == 0 {
                    0.0
                } else {
                    (frame as f64 / 4.0).min(1.0)
                };
                (0..channels).map(move |channel| {
                    let balance = if channels == 2 && channel == 0 {
                        1.0 - progress
                    } else {
                        1.0
                    };
                    0.375 * (1.0 - progress) * balance
                })
            })
            .collect::<Vec<_>>();
        assert_samples(&samples, &expected);
    }
}

#[test]
fn send_attenuation_precedes_gain_overflow() {
    for (pan, expected_left) in [(1.0, 0.0), (0.5, f64::MAX)] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        connect(
            &mut graph,
            source,
            sink,
            SendParams {
                gain: f64::MAX,
                pan,
                ..SendParams::default()
            },
        );
        let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 2.0,
                },
            )
            .unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        compiled.engine.render(1).unwrap();
        let mut samples = [0.0; 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples[0], expected_left);
        assert_eq!(samples[1], f64::INFINITY);
    }
}

#[test]
fn bound_io_rejects_reversed_roles_and_mismatched_channels() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, sink, SendParams::default());
    let (writer, _) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut reversed = NodeBindings::new();
    reversed.bind_sink(source, writer).unwrap();
    reversed
        .bind_source(
            sink,
            ConstantSource {
                channels: 2,
                value: 1.0,
            },
        )
        .unwrap();
    assert!(matches!(
        compile(&graph, reversed, config()),
        Err(CompileError::BindingKind { .. })
    ));
    let (writer, _) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut mismatched = NodeBindings::new();
    mismatched
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 1.0,
            },
        )
        .unwrap();
    mismatched.bind_sink(sink, writer).unwrap();
    assert!(matches!(
        compile(&graph, mismatched, config()),
        Err(CompileError::Runtime(RuntimeError::Processor(
            ProcessorError::InvalidIo
        )))
    ));
}

#[test]
fn prepared_io_nodes_keep_their_boundary_telemetry_through_compilation() {
    use moiren_engine::node::*;
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, sink, SendParams::default());
    let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let (input_node, mut input_reports) = InputNode::new(
        InputConfig {
            source: InputSource::Software {
                name: "compiled input".into(),
            },
            channels: 2,
            device: DeviceOptions::default(),
        },
        ConstantSource {
            channels: 2,
            value: 0.25,
        },
        8,
    )
    .unwrap();
    let (output_node, mut output_reports) = OutputNode::new(
        OutputConfig {
            target: OutputTarget::Software {
                name: "compiled output".into(),
            },
            channels: 2,
            device: DeviceOptions::default(),
        },
        writer,
        8,
    )
    .unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_io(source, input_node).unwrap();
    bindings.bind_io(sink, output_node).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    for frames in [3, 1, 4] {
        compiled.engine.render(frames).unwrap();
    }
    let mut samples = [0.0; 16];
    reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.25; 16]);
    for snapshot in [
        input_reports.latest().unwrap(),
        output_reports.latest().unwrap(),
    ] {
        assert_eq!(snapshot.timeline_epoch, 3);
        assert_eq!(snapshot.end_frame, 8);
        assert_eq!(snapshot.total_transferred_frames, 8);
        assert_eq!(snapshot.total_xruns, 0);
    }
}
