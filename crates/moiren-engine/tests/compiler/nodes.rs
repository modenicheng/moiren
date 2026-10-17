use super::*;

fn compile_transform(kind: NodeKind) -> CompiledGraph<f64> {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let transform = graph.create_node(kind, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, transform, SendParams::default());
    connect(&mut graph, transform, sink, SendParams::default());
    let (writer, _) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    match kind {
        NodeKind::Eq => bindings.bind_eq(transform, EqSettings::default()).unwrap(),
        NodeKind::Gate => bindings
            .bind_gate(transform, GateSettings::default())
            .unwrap(),
        NodeKind::Limiter => bindings
            .bind_limiter(transform, LimiterSettings::default())
            .unwrap(),
        NodeKind::Delay => bindings
            .bind_delay(transform, DelaySettings::default())
            .unwrap(),
        NodeKind::ChannelMapper
        | NodeKind::Recorder
        | NodeKind::Meter
        | NodeKind::SpectrumAnalyzer
        | NodeKind::Rack => bindings.bind_utility(transform).unwrap(),
        _ => unreachable!("test only covers new transform nodes"),
    }
    compile(&graph, bindings, config()).unwrap()
}

#[test]
fn planned_transform_nodes_lower_into_a_runnable_execution_plan() {
    for kind in [
        NodeKind::Eq,
        NodeKind::Gate,
        NodeKind::Limiter,
        NodeKind::Delay,
        NodeKind::ChannelMapper,
        NodeKind::Recorder,
        NodeKind::Meter,
        NodeKind::SpectrumAnalyzer,
        NodeKind::Rack,
    ] {
        let mut compiled = compile_transform(kind);
        compiled.engine.render(4).unwrap();
    }
}
