use super::*;

#[test]
fn rack_strip_invalid_auxiliary_channel_is_rejected_before_lowering() {
    use moiren_engine::rack::RackSettings;
    let mut graph = LogicalGraph::new();
    let rack = graph.create_rack(&[2, 1], &[2, 1]).unwrap();
    let mut bindings = NodeBindings::<f64>::new();
    bindings.bind_rack(rack, RackSettings::default()).unwrap();
    bindings.bind_strip(
        rack,
        StripConfig {
            mode: ChannelMode::Solo(1),
            ..Default::default()
        },
    );
    assert!(matches!(
        compile(&graph, bindings, config()),
        Err(CompileError::BindingKind { node }) if node == rack
    ));
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

#[test]
fn virtual_io_contracts_compile_with_portable_boundaries_and_render_without_allocations() {
    use moiren_engine::node::*;

    fn check<S: ProcessingSample>(channels: usize) {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        connect(&mut graph, source, sink, SendParams::default());
        let (mut producer, source_audio) = audio_bridge::<S>(channels, 8, 4096).unwrap();
        let (sink_audio, mut consumer) = audio_bridge::<S>(channels, 8, 4096).unwrap();
        let (input_node, mut input_reports) = InputNode::new(
            InputConfig {
                source: InputSource::VirtualPlayback {
                    endpoint_id: VirtualEndpointId::from_bytes([1; 16]),
                },
                channels,
                device: Default::default(),
            },
            source_audio,
            8,
        )
        .unwrap();
        let (output_node, mut output_reports) = OutputNode::new(
            OutputConfig {
                target: OutputTarget::VirtualMicrophone {
                    endpoint_id: VirtualEndpointId::from_bytes([2; 16]),
                },
                channels,
                device: Default::default(),
            },
            sink_audio,
            8,
        )
        .unwrap();
        let mut bindings = NodeBindings::new();
        bindings.bind_io(source, input_node).unwrap();
        bindings.bind_io(sink, output_node).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let samples: Vec<S> = (0..5 * channels)
            .map(|index| S::from_f64((index + 1) as f64 / 16.0))
            .collect();
        producer.write_interleaved(&samples).unwrap();
        let mut output = vec![S::from_f64(99.0); 8 * channels];
        let ((first, second, report), counts) = allocation::track_allocations(|| {
            let first = compiled.engine.render(3);
            let second = compiled.engine.render(5);
            let report = consumer.read_interleaved(&mut output);
            (first, second, report)
        });
        assert_eq!(counts, (0, 0));
        first.unwrap();
        second.unwrap();
        assert_eq!(report.unwrap().transferred_frames, 8);
        for (&actual, &expected) in output.iter().zip(&samples) {
            assert_eq!(actual.to_f64(), expected.to_f64());
        }
        assert!(
            output[samples.len()..]
                .iter()
                .all(|&value| value.to_f64() == 0.0)
        );
        let input = input_reports.latest().unwrap();
        assert_eq!(input.total_transferred_frames, 5);
        assert_eq!(input.total_shortfall_frames, 3);
        let output = output_reports.latest().unwrap();
        assert_eq!(output.total_transferred_frames, 8);
        assert_eq!(output.total_shortfall_frames, 0);
    }

    for channels in [1, 2] {
        check::<f32>(channels);
        check::<f64>(channels);
    }
}
