use super::*;

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
    for kind in [
        NodeKind::Bus,
        NodeKind::Gain,
        NodeKind::Pan,
        NodeKind::Compressor,
        NodeKind::Sink,
    ] {
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
