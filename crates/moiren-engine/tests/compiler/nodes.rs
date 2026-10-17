use super::*;

#[test]
fn configured_channel_matrix_downmixes_without_implicit_conversion() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let mapper = graph.create_channel_mapper(2, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, mapper, Default::default());
    connect(&mut graph, mapper, sink, Default::default());
    let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_channel_mapper(
            mapper,
            ChannelMapperSettings {
                matrix: vec![vec![0.25, 0.5]],
                ..Default::default()
            },
        )
        .unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let (_, heap) = allocation::track_heap(|| compiled.engine.render(4).unwrap());
    assert_eq!(heap, (0, 0, 0));
    let mut samples = [0.0; 4];
    reader.read_interleaved(&mut samples).unwrap();
    assert_samples(&samples, &[0.28125; 4]);
}

#[test]
fn measurement_and_recording_nodes_require_real_bindings() {
    for kind in [
        NodeKind::Meter,
        NodeKind::Recorder,
        NodeKind::SpectrumAnalyzer,
    ] {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(kind, 1).unwrap();
        assert!(matches!(
            compile::<f64>(&graph, NodeBindings::new(), config()),
            Err(CompileError::MissingBinding { node: missing }) if missing == node
        ));
    }
}

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
        NodeKind::ChannelMapper => bindings
            .bind_channel_mapper(transform, ChannelMapperSettings::default())
            .unwrap(),
        NodeKind::Meter => bindings
            .bind_meter(transform, moiren_engine::meter::MeterSettings::default())
            .map(|_| ())
            .unwrap(),
        NodeKind::Recorder => bindings
            .bind_recorder(
                transform,
                1,
                moiren_engine::recorder::RecorderSettings::default(),
            )
            .map(|_| ())
            .unwrap(),
        NodeKind::SpectrumAnalyzer | NodeKind::Rack => {
            panic!("test requires a prepared signal or rack binding")
        }
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
