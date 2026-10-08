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
fn bus_input_count_fits_engine_port_indices() {
    let mut graph = LogicalGraph::new();
    let bus = graph.create_node(NodeKind::Bus, 1).unwrap();
    for _ in 0..MAX_INPUT_PORTS {
        graph.add_input_port(bus, 1).unwrap();
    }
    let before = graph.clone();
    assert_eq!(graph.add_input_port(bus, 1), Err(GraphError::TooManyPorts));
    assert_eq!(graph, before);
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
