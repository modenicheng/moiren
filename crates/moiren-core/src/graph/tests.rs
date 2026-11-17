use super::*;

#[test]
fn id_exhaustion_is_atomic_for_nodes_ports_and_edges() {
    let mut graph = LogicalGraph::new();
    graph.counts.node = u64::MAX;
    let before = graph.clone();
    assert_eq!(
        graph.create_node(NodeKind::Source, 2),
        Err(GraphError::TooManyNodes)
    );
    assert_eq!(graph, before);

    graph.counts.node = 0;
    graph.counts.port = u64::MAX - 1;
    let before = graph.clone();
    assert_eq!(
        graph.create_node(NodeKind::Gain, 2),
        Err(GraphError::TooManyPorts)
    );
    assert_eq!(graph, before);
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let before = graph.clone();
    assert_eq!(graph.add_input_port(bus, 2), Err(GraphError::TooManyPorts));
    assert_eq!(graph, before);

    let mut graph = LogicalGraph::new();
    let src = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let src_port = graph.get_node(src).unwrap().outputs()[0].id();
    let dst_port = graph.add_input_port(bus, 2).unwrap();
    graph.counts.edge = u64::MAX;
    let before = graph.clone();
    assert_eq!(
        graph.connect(src_port, dst_port, SendParams::default()),
        Err(GraphError::TooManyEdges)
    );
    assert_eq!(graph, before);
    assert_eq!(
        edit::connect_to_new_bus_input(&mut graph, src_port, bus, SendParams::default()),
        Err(GraphError::TooManyEdges)
    );
    assert_eq!(graph.nodes, before.nodes);
    assert_eq!(graph.edges, before.edges);
    graph.validate().unwrap();
}

#[test]
fn connect_rejections_preserve_graph_state_across_every_rule() {
    let mut graph = LogicalGraph::new();
    let first = graph.create_node(NodeKind::Gain, 1).unwrap();
    let second = graph.create_node(NodeKind::Gain, 1).unwrap();
    let stereo = graph.create_node(NodeKind::Gain, 2).unwrap();
    let first_input = graph.get_node(first).unwrap().inputs()[0].id();
    let first_output = graph.get_node(first).unwrap().outputs()[0].id();
    let second_input = graph.get_node(second).unwrap().inputs()[0].id();
    let second_output = graph.get_node(second).unwrap().outputs()[0].id();
    let stereo_input = graph.get_node(stereo).unwrap().inputs()[0].id();
    graph
        .connect(first_output, second_input, SendParams::default())
        .unwrap();
    for (output, input, params, error) in [
        (
            second_input,
            first_output,
            SendParams::default(),
            GraphError::PortDirectionMismatch,
        ),
        (
            first_output,
            second_input,
            SendParams::default(),
            GraphError::InputAlreadyConnected,
        ),
        (
            second_output,
            first_input,
            SendParams::default(),
            GraphError::CycleDetected,
        ),
        (
            first_output,
            stereo_input,
            SendParams::default(),
            GraphError::ChannelMismatch,
        ),
        (
            second_output,
            first_input,
            SendParams {
                gain: f64::NAN,
                ..Default::default()
            },
            GraphError::InvalidSendParameters,
        ),
        (
            PortId(u64::MAX),
            first_input,
            SendParams::default(),
            GraphError::PortNotFound,
        ),
    ] {
        let original = graph.clone();
        assert_eq!(graph.connect(output, input, params), Err(error));
        assert_eq!(graph, original);
    }
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let output = graph.get_node(source).unwrap().outputs()[0].id();
    assert!(
        graph
            .connect(output, first_input, SendParams::default())
            .is_ok()
    );
}

#[test]
fn bus_input_count_fits_engine_port_indices() {
    // The u16 boundary fill itself is covered by the native suite; Miri still
    // checks dynamic add/remove/reuse, just without 65k live port records.
    const FILLED: usize = if cfg!(miri) { 256 } else { MAX_INPUT_PORTS };
    let mut graph = LogicalGraph::new();
    let bus = graph.create_node(NodeKind::Bus, 1).unwrap();
    for _ in 0..FILLED {
        graph.add_input_port(bus, 1).unwrap();
    }
    if FILLED == MAX_INPUT_PORTS {
        let before = graph.clone();
        assert_eq!(graph.add_input_port(bus, 1), Err(GraphError::TooManyPorts));
        assert_eq!(graph, before);
    }
    let first = graph.get_node(bus).unwrap().inputs()[0].id();
    graph.remove_input_port(bus, first).unwrap();
    graph.add_input_port(bus, 1).unwrap();
    graph.validate().unwrap();
}

#[test]
fn validation_detects_corrupt_layout_and_cycle_at_the_compiler_boundary() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Gain, 2).unwrap();
    let clean = graph.clone();
    graph.nodes[0].inputs[0].channels = 1;
    assert_eq!(graph.validate(), Err(GraphError::InvalidNodeLayout));
    graph = clean;
    graph.edges.push(Edge {
        id: EdgeId(0),
        src: node,
        dst: node,
        src_port: graph.nodes[0].outputs[0].id,
        dst_port: graph.nodes[0].inputs[0].id,
        params: SendParams::default(),
    });
    assert_eq!(graph.validate(), Err(GraphError::CycleDetected));
}

/// Resizing a width-mirroring node rewrites its ports in place: identities
/// survive, edges compatible with the new width keep flowing, and the next
/// validate agrees. A wired node whose neighbors cannot follow is rejected
/// wholesale; disconnecting the stranded routes frees the resize.
#[test]
fn set_node_channels_resizes_ports_in_place_and_keeps_compatible_edges() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let plugin = graph.create_node(NodeKind::Plugin, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let in_port = graph.get_node(plugin).unwrap().inputs()[0].id();
    let out_port = graph.get_node(plugin).unwrap().outputs()[0].id();
    let edge_in = graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            in_port,
            SendParams::default(),
        )
        .unwrap();
    let edge_out = graph
        .connect(
            out_port,
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();

    // Widening only works when every far side can follow: both neighbors
    // are 2ch here, so the whole resize is rejected and nothing moved.
    assert_eq!(
        graph.set_node_channels(plugin, 6),
        Err(GraphError::StrandedEdges)
    );
    assert_eq!(graph.get_node(plugin).unwrap().channels(), 2);
    assert_eq!(graph.validate(), Ok(()));

    // Disconnecting the stranded routes frees the resize; port identities
    // survive and the widened ports accept fresh equal-width wiring.
    graph.disconnect(edge_in).unwrap();
    graph.disconnect(edge_out).unwrap();
    graph.set_node_channels(plugin, 6).unwrap();
    assert_eq!(graph.get_node(plugin).unwrap().channels(), 6);
    assert_eq!(
        graph.get_node(plugin).unwrap().inputs()[0].id(),
        in_port,
        "port identities survive the resize"
    );
    assert_eq!(graph.get_node(plugin).unwrap().inputs()[0].channels(), 6);
    assert_eq!(graph.get_node(plugin).unwrap().outputs()[0].channels(), 6);
    graph.set_node_channels(source, 6).unwrap();
    graph.set_node_channels(sink, 6).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            in_port,
            SendParams::default(),
        )
        .unwrap();
    graph
        .connect(
            out_port,
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    graph.validate().unwrap();

    // Reconnected at the new width, everything flows again — and any real
    // resize strands its incident edges by the equal-width invariant (only
    // a no-op resize can keep one), so a wired node must be freed first.
    assert_eq!(
        graph.set_node_channels(plugin, 4),
        Err(GraphError::StrandedEdges)
    );
    graph.validate().unwrap();
}

/// The resize contract's boundaries: fixed-stream layouts are rejected,
/// kind width rules still hold, and the graph survives untouched.
#[test]
fn set_node_channels_rejects_non_mirroring_layouts_and_invalid_widths() {
    let mut graph = LogicalGraph::new();
    let rack = graph.create_rack(&[1, 2], &[2, 1]).expect("rack streams");
    let mapper = graph.create_channel_mapper(1, 2).expect("mapper");
    let pan = graph.create_node(NodeKind::Pan, 2).unwrap();
    let before = graph.clone();
    assert_eq!(
        graph.set_node_channels(rack, 4),
        Err(GraphError::FixedPortLayout)
    );
    assert_eq!(
        graph.set_node_channels(mapper, 4),
        Err(GraphError::FixedPortLayout)
    );
    // Pan is fixed stereo by kind rule; zero is never a width.
    assert_eq!(
        graph.set_node_channels(pan, 4),
        Err(GraphError::InvalidChannelCount)
    );
    assert_eq!(
        graph.set_node_channels(pan, 0),
        Err(GraphError::InvalidChannelCount)
    );
    assert_eq!(
        graph.set_node_channels(NodeId(u64::MAX), 2),
        Err(GraphError::NodeNotFound)
    );
    assert_eq!(graph, before, "every rejection leaves the graph untouched");

    // Bus ports mirror the width, so the bus itself resizes.
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    graph.add_input_port(bus, 2).unwrap();
    graph.set_node_channels(bus, 4).unwrap();
    let bus_node = graph.get_node(bus).unwrap();
    assert!(
        bus_node
            .inputs()
            .iter()
            .chain(bus_node.outputs())
            .all(|port| port.channels() == 4)
    );
    graph.validate().unwrap();
}
