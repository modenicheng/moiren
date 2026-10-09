use moiren_core::graph::{edit::connect_to_new_bus_input, *};

fn input(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().inputs()[0].id()
}

fn output(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().outputs()[0].id()
}

#[test]
fn node_kinds_create_fixed_layouts_and_bus_starts_without_inputs() {
    let mut graph = LogicalGraph::new();
    for (kind, channels, inputs, outputs) in [
        (NodeKind::Source, 6, 0, 1),
        (NodeKind::Sink, 6, 1, 0),
        (NodeKind::Gain, 6, 1, 1),
        (NodeKind::Compressor, 6, 1, 1),
        (NodeKind::Bus, 6, 0, 1),
        (NodeKind::Pan, 2, 1, 1),
    ] {
        let id = graph.create_node(kind, channels).unwrap();
        let node = graph.get_node(id).unwrap();
        assert_eq!(node.id(), id);
        assert_eq!(node.kind(), kind);
        assert_eq!(node.channels(), channels);
        assert_eq!(
            (node.inputs().len(), node.outputs().len()),
            (inputs, outputs)
        );
        for port in node.inputs().iter().chain(node.outputs()) {
            assert_eq!(port.channels(), channels);
            assert_eq!(graph.get_port(port.id()).unwrap(), port);
        }
        assert!(node.inputs().iter().all(|p| p.role() == PortRole::Input));
        assert!(node.outputs().iter().all(|p| p.role() == PortRole::Output));
    }
    assert_eq!(
        graph.create_node(NodeKind::Bus, 0),
        Err(GraphError::InvalidChannelCount)
    );
    assert_eq!(
        graph.create_node(NodeKind::Pan, 1),
        Err(GraphError::InvalidChannelCount)
    );
    assert_eq!(graph.nodes().len(), 6);
    graph.validate().unwrap();
}

#[test]
fn only_bus_inputs_are_dynamic_and_channels_match_the_output() {
    let mut graph = LogicalGraph::default();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let a = graph.add_input_port(bus, 2).unwrap();
    let b = graph.add_input_port(bus, 2).unwrap();
    assert_ne!(a, b);
    assert_eq!(
        graph.add_input_port(bus, 0),
        Err(GraphError::InvalidChannelCount)
    );
    assert_eq!(
        graph.add_input_port(bus, 1),
        Err(GraphError::ChannelMismatch)
    );
    for kind in [
        NodeKind::Source,
        NodeKind::Sink,
        NodeKind::Gain,
        NodeKind::Compressor,
        NodeKind::Pan,
    ] {
        let id = graph.create_node(kind, 2).unwrap();
        assert_eq!(
            graph.add_input_port(id, 2),
            Err(GraphError::FixedPortLayout)
        );
    }
    graph.remove_input_port(bus, a).unwrap();
    assert_eq!(graph.get_port(a), Err(GraphError::PortNotFound));
    assert_eq!(graph.get_node(bus).unwrap().inputs()[0].id(), b);
    assert_ne!(graph.add_input_port(bus, 2).unwrap(), a);
    graph.remove_node(bus).unwrap();
    assert_eq!(graph.add_input_port(bus, 2), Err(GraphError::NodeNotFound));
    graph.validate().unwrap();
}

#[test]
fn connections_allow_fan_out_and_repeated_bus_sources_but_reject_fan_in() {
    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let other = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let a = graph.add_input_port(bus, 2).unwrap();
    let b = graph.add_input_port(bus, 2).unwrap();
    let src_port = output(&graph, src);
    let other_port = output(&graph, other);
    let sink_port = input(&graph, sink);
    let params = SendParams {
        gain: 0.5,
        pan: -0.25,
        mute: true,
        tap: SendTap::PreFader,
    };
    let edge = graph.connect(src_port, a, params).unwrap();
    graph.connect(src_port, b, SendParams::default()).unwrap();
    graph
        .connect(src_port, sink_port, SendParams::default())
        .unwrap();
    assert_eq!(
        graph.connect(other_port, a, params),
        Err(GraphError::InputAlreadyConnected)
    );
    assert_eq!(
        graph.connect(src_port, a, params),
        Err(GraphError::InputAlreadyConnected)
    );
    let edge = graph.get_edge(edge).unwrap();
    assert_eq!(
        (edge.src(), edge.dst(), edge.src_port(), edge.dst_port()),
        (src, bus, src_port, a)
    );
    assert_eq!(*edge.params(), params);
    assert_eq!(graph.edges().len(), 3);
    graph.validate().unwrap();
}

#[test]
fn connect_rejects_wrong_direction_channels_and_nonexistent_ports_without_mutation() {
    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let mono = graph.create_node(NodeKind::Sink, 1).unwrap();
    let src_port = output(&graph, src);
    let sink_port = input(&graph, sink);
    for (a, b) in [
        (sink_port, src_port),
        (src_port, src_port),
        (sink_port, sink_port),
    ] {
        assert_eq!(
            graph.connect(a, b, SendParams::default()),
            Err(GraphError::PortDirectionMismatch)
        );
    }
    assert_eq!(
        graph.connect(src_port, input(&graph, mono), SendParams::default()),
        Err(GraphError::ChannelMismatch)
    );
    graph.remove_node(mono).unwrap();
    graph.remove_node(src).unwrap();
    assert_eq!(
        graph.connect(src_port, sink_port, SendParams::default()),
        Err(GraphError::PortNotFound)
    );
    assert!(graph.edges().is_empty());
}

#[test]
fn cycle_detection_handles_self_loops_indirect_cycles_and_disconnected_components() {
    let mut graph = LogicalGraph::new();
    let a = graph.create_node(NodeKind::Gain, 2).unwrap();
    let b = graph.create_node(NodeKind::Gain, 2).unwrap();
    let c = graph.create_node(NodeKind::Gain, 2).unwrap();
    let d = graph.create_node(NodeKind::Gain, 2).unwrap();
    assert_eq!(
        graph.connect(output(&graph, a), input(&graph, a), SendParams::default()),
        Err(GraphError::CycleDetected)
    );
    graph
        .connect(output(&graph, a), input(&graph, b), SendParams::default())
        .unwrap();
    graph
        .connect(output(&graph, b), input(&graph, c), SendParams::default())
        .unwrap();
    assert_eq!(
        graph.connect(output(&graph, c), input(&graph, a), SendParams::default()),
        Err(GraphError::CycleDetected)
    );
    graph
        .connect(output(&graph, c), input(&graph, d), SendParams::default())
        .unwrap();
    assert_eq!(graph.topological_order().unwrap(), [a, b, c, d]);
    graph.validate().unwrap();
}

#[test]
fn stable_topological_order_counts_parallel_edges_and_includes_isolated_nodes() {
    let mut graph = LogicalGraph::new();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let isolated = graph.create_node(NodeKind::Sink, 1).unwrap();
    let a = graph.add_input_port(bus, 2).unwrap();
    let b = graph.add_input_port(bus, 2).unwrap();
    graph
        .connect(output(&graph, source), a, SendParams::default())
        .unwrap();
    graph
        .connect(output(&graph, source), b, SendParams::default())
        .unwrap();
    assert_eq!(graph.topological_order().unwrap(), [source, bus, isolated]);
    assert_eq!(LogicalGraph::new().topological_order().unwrap(), []);
}

#[test]
fn disconnect_keeps_bus_ports_and_node_deletion_removes_incident_edges() {
    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let port = graph.add_input_port(bus, 2).unwrap();
    let edge = graph
        .connect(output(&graph, src), port, SendParams::default())
        .unwrap();
    assert_eq!(
        graph.remove_input_port(bus, port),
        Err(GraphError::PortConnected)
    );
    let removed = graph.disconnect(edge).unwrap();
    assert_eq!(removed.id(), edge);
    assert_eq!(graph.disconnect(edge), Err(GraphError::EdgeNotFound));
    assert_eq!(graph.get_node(bus).unwrap().inputs().len(), 1);
    let replacement = graph
        .connect(output(&graph, src), port, SendParams::default())
        .unwrap();
    assert_ne!(replacement, edge);
    graph
        .connect(
            output(&graph, bus),
            input(&graph, sink),
            SendParams::default(),
        )
        .unwrap();
    graph.remove_node(bus).unwrap();
    assert!(graph.edges().is_empty());
    assert_eq!(graph.get_edge(replacement), Err(GraphError::EdgeNotFound));
    assert_eq!(graph.get_node(bus), Err(GraphError::NodeNotFound));
    assert_eq!(graph.get_port(port), Err(GraphError::PortNotFound));
    let new_bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    assert_ne!(new_bus, bus);
    graph.validate().unwrap();
}

#[test]
fn send_updates_validate_values_and_preserve_topology() {
    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let src_port = output(&graph, src);
    let sink_port = input(&graph, sink);
    for params in [
        SendParams {
            gain: f64::NAN,
            ..SendParams::default()
        },
        SendParams {
            gain: f64::INFINITY,
            ..SendParams::default()
        },
        SendParams {
            gain: -0.1,
            ..SendParams::default()
        },
        SendParams {
            pan: f64::NAN,
            ..SendParams::default()
        },
        SendParams {
            pan: f64::INFINITY,
            ..SendParams::default()
        },
        SendParams {
            pan: -1.1,
            ..SendParams::default()
        },
        SendParams {
            pan: 1.1,
            ..SendParams::default()
        },
    ] {
        assert_eq!(
            graph.connect(src_port, sink_port, params),
            Err(GraphError::InvalidSendParameters)
        );
    }
    let edge = graph
        .connect(src_port, sink_port, SendParams::default())
        .unwrap();
    let params = SendParams {
        gain: 2.0,
        pan: 1.0,
        mute: true,
        tap: SendTap::PreFader,
    };
    graph.set_send_params(edge, params).unwrap();
    assert_eq!(
        graph.set_send_params(edge, SendParams { pan: 2.0, ..params }),
        Err(GraphError::InvalidSendParameters)
    );
    assert_eq!(*graph.get_edge(edge).unwrap().params(), params);
    assert_eq!(graph.topological_order().unwrap(), [src, sink]);
    assert_eq!(graph.edges().len(), 1);
}

#[test]
fn composed_bus_edit_rolls_back_port_on_failed_connection() {
    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let mono = graph.create_node(NodeKind::Source, 1).unwrap();
    let mono_port = output(&graph, mono);
    let bus_output = output(&graph, bus);
    for (port, params, error) in [
        (
            mono_port,
            SendParams::default(),
            GraphError::ChannelMismatch,
        ),
        (bus_output, SendParams::default(), GraphError::CycleDetected),
        (
            output(&graph, src),
            SendParams {
                gain: -1.0,
                ..SendParams::default()
            },
            GraphError::InvalidSendParameters,
        ),
    ] {
        assert_eq!(
            connect_to_new_bus_input(&mut graph, port, bus, params),
            Err(error)
        );
        assert!(graph.get_node(bus).unwrap().inputs().is_empty());
        assert!(graph.edges().is_empty());
    }
    graph.remove_node(mono).unwrap();
    assert_eq!(
        connect_to_new_bus_input(&mut graph, mono_port, bus, SendParams::default()),
        Err(GraphError::PortNotFound)
    );
    let src_port = output(&graph, src);
    let edge = connect_to_new_bus_input(&mut graph, src_port, bus, SendParams::default()).unwrap();
    assert_eq!(graph.get_node(bus).unwrap().inputs().len(), 1);
    assert_eq!(graph.get_edge(edge).unwrap().dst_port(), input(&graph, bus));
    graph.validate().unwrap();
}

#[test]
fn deep_chains_use_iterative_traversal_and_remain_reconnectable() {
    let mut graph = LogicalGraph::new();
    let nodes = (0..512)
        .map(|_| graph.create_node(NodeKind::Gain, 1).unwrap())
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    for pair in nodes.windows(2) {
        edges.push(
            graph
                .connect(
                    output(&graph, pair[0]),
                    input(&graph, pair[1]),
                    SendParams::default(),
                )
                .unwrap(),
        );
    }
    let last = output(&graph, *nodes.last().unwrap());
    let first = input(&graph, nodes[0]);
    assert_eq!(
        graph.connect(last, first, SendParams::default()),
        Err(GraphError::CycleDetected)
    );
    assert_eq!(graph.topological_order().unwrap(), nodes);
    graph.disconnect(edges[255]).unwrap();
    graph.connect(last, first, SendParams::default()).unwrap();
    graph.validate().unwrap();
}
