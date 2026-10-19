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
    assert_samples_close(&samples, &[0.28125; 4]);
}

#[test]
fn configured_channel_matrix_uses_distinct_input_and_output_layouts() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let mapper = graph.create_channel_mapper(1, 3).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 3).unwrap();
    connect(&mut graph, source, mapper, Default::default());
    connect(&mut graph, mapper, sink, Default::default());
    let (writer, mut reader) = audio_bridge::<f64>(3, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_channel_mapper(
            mapper,
            ChannelMapperSettings {
                matrix: vec![vec![0.25], vec![-1.0], vec![2.0]],
                ..Default::default()
            },
        )
        .unwrap();

    let mut compiled = compile(&graph, bindings, config()).unwrap();
    compiled.engine.render(4).unwrap();
    let mut samples = [0.0; 12];
    reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(
        &samples,
        &[
            0.09375, -0.375, 0.75, 0.09375, -0.375, 0.75, 0.09375, -0.375, 0.75, 0.09375, -0.375,
            0.75,
        ],
    );
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
        NodeKind::Rack => bindings
            .bind_rack(transform, moiren_engine::rack::RackSettings::default())
            .unwrap(),
        NodeKind::SpectrumAnalyzer => panic!("test requires a prepared signal binding"),
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
        NodeKind::Rack,
        NodeKind::Recorder,
        NodeKind::Meter,
    ] {
        let mut compiled = compile_transform(kind);
        compiled.engine.render(4).unwrap();
    }
}

#[test]
fn rack_lowering_runs_empty_and_active_chains_and_resolves_slot_parameters() {
    use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};

    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let rack = graph.create_node(NodeKind::Rack, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, rack, Default::default());
    connect(&mut graph, rack, sink, Default::default());
    let make_bindings = |settings| {
        let (writer, reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 1);
        bindings.bind_sink(sink, writer).unwrap();
        bindings.bind_rack(rack, settings).unwrap();
        (bindings, reader)
    };

    let (bindings, mut empty_reader) = make_bindings(RackSettings::default());
    let mut empty = compile(&graph, bindings, config()).unwrap();
    empty.engine.render(2).unwrap();
    let mut samples = [0.0; 2];
    empty_reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(&samples, &[0.375, 0.375]);

    let settings = RackSettings {
        slots: vec![
            RackSlot {
                id: 90,
                bypass: false,
                processor: RackProcessor::Gain { initial: 2.0 },
            },
            RackSlot {
                id: 12,
                bypass: false,
                processor: RackProcessor::Gain { initial: 0.5 },
            },
        ],

        ..Default::default()
    };
    let (bindings, mut reader) = make_bindings(settings.clone());
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let first_id = compiled.bindings.rack_slot(rack, 90).unwrap();
    let second_id = compiled.bindings.rack_slot(rack, 12).unwrap();
    assert_ne!(first_id, second_id);
    assert!(compiled.bindings.rack_slot(rack, 999).is_none());

    let response = compiled.control.submit(
        ParameterRequest {
            request_id: 4,
            plan_revision: 7,
            timeline_epoch: 3,
            target: ParameterKey {
                processor: first_id,
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::NextBlock,
            value: ParamValue::Float(0.25),
            ramp_frames: 0,
        },
        0,
    );
    assert_eq!(response.code, ReplyCode::Accepted);
    compiled.engine.render(2).unwrap();
    reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(&samples, &[0.046875, 0.046875]);

    let (bindings, mut bypass_reader) = make_bindings(RackSettings {
        slots: vec![
            RackSlot {
                bypass: true,
                ..settings.slots[0].clone()
            },
            settings.slots[1].clone(),
        ],

        ..Default::default()
    });
    let mut bypassed = compile(&graph, bindings, config()).unwrap();
    assert_eq!(bypassed.bindings.rack_slot(rack, 90), None);
    let bypassed_slot = bypassed.bindings.rack_slot(rack, 12).unwrap();
    let response = bypassed.control.submit(
        ParameterRequest {
            request_id: 5,
            plan_revision: 7,
            timeline_epoch: 3,
            target: ParameterKey {
                processor: bypassed_slot,
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::NextBlock,
            value: ParamValue::Float(0.25),
            ramp_frames: 0,
        },
        0,
    );
    assert_eq!(response.code, ReplyCode::Accepted);
    bypassed.engine.render(2).unwrap();
    bypass_reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(&samples, &[0.09375, 0.09375]);

    let (bindings, mut reordered_reader) = make_bindings(RackSettings {
        slots: settings.slots.iter().rev().cloned().collect(),

        ..Default::default()
    });
    let mut reordered = compile(&graph, bindings, config()).unwrap();
    let reordered_slot = reordered.bindings.rack_slot(rack, 90).unwrap();
    let response = reordered.control.submit(
        ParameterRequest {
            request_id: 6,
            plan_revision: 7,
            timeline_epoch: 3,
            target: ParameterKey {
                processor: reordered_slot,
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::NextBlock,
            value: ParamValue::Float(0.25),
            ramp_frames: 0,
        },
        0,
    );
    assert_eq!(response.code, ReplyCode::Accepted);
    reordered.engine.render(2).unwrap();
    reordered_reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(&samples, &[0.046875, 0.046875]);
}

#[test]
fn planned_signal_nodes_require_an_explicit_observer_binding() {
    let mut graph = LogicalGraph::new();
    let analyzer = graph.create_node(NodeKind::SpectrumAnalyzer, 1).unwrap();
    assert!(matches!(
        compile::<f64>(&graph, NodeBindings::new(), config()),
        Err(CompileError::MissingBinding { node }) if node == analyzer
    ));
}

#[test]
fn spectrum_node_lowers_with_an_explicit_publisher_binding() {
    use moiren_engine::spectrum::{SignalSpectrumAnalyzer, SpectrumSettings, SpectrumWindow};
    use moiren_signal::{BlockInit, Bus};

    let settings = SpectrumSettings {
        fft_size: 16,
        hop_frames: 8,
    };
    let mut signal_bus = Bus::new(Default::default()).unwrap();
    let topic = signal_bus
        .register_latest_block(
            "compiler/nodes/spectrum",
            BlockInit {
                header: SpectrumWindow {
                    timeline_epoch: 0,
                    frame_start: 0,
                    frame_end: 0,
                    sample_rate: 0.0,
                    fft_size: 0,
                    channels: 0,
                },
                element: 0.0_f32,
                capacity: settings.bins_per_channel(),
            },
        )
        .unwrap();
    let (host_control, host) = signal_bus.create_host(4).unwrap();
    let publisher = host_control
        .prepare(&topic, host_control.new_binding().unwrap())
        .unwrap();

    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let analyzer = graph.create_node(NodeKind::SpectrumAnalyzer, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, analyzer, Default::default());
    connect(&mut graph, analyzer, sink, Default::default());
    let (writer, _) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_spectrum(
            analyzer,
            SignalSpectrumAnalyzer::new(1, settings, publisher).unwrap(),
        )
        .unwrap();

    let mut compiled = compile_with_signals(&graph, bindings, config(), &host_control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    compiled.engine.render(8).unwrap();
}
