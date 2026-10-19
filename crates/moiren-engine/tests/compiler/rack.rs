use super::*;
use moiren_engine::rack::{RackProcessor, RackRoute, RackSettings, RackSlot};

#[test]
fn rack_routes_multiple_widths_and_mapper_slots_without_render_allocations() {
    fn check<S: ProcessingSample>() {
        let mut graph = LogicalGraph::new();
        let stereo_source = graph.create_node(NodeKind::Source, 2).unwrap();
        let mono_source = graph.create_node(NodeKind::Source, 1).unwrap();
        let rack = graph.create_node(NodeKind::Rack, 2).unwrap();
        let mono_input = graph.add_input_port(rack, 1).unwrap();
        let auxiliary_output = graph.add_output_port(rack, 2).unwrap();
        let stereo_sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let auxiliary_sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let stereo_input = graph.get_node(rack).unwrap().inputs()[0].id();
        let stereo_output = graph.get_node(rack).unwrap().outputs()[0].id();
        graph
            .connect(
                output(&graph, stereo_source),
                stereo_input,
                SendParams::default(),
            )
            .unwrap();
        graph
            .connect(
                output(&graph, mono_source),
                mono_input,
                SendParams::default(),
            )
            .unwrap();
        graph
            .connect(
                stereo_output,
                input(&graph, stereo_sink),
                SendParams::default(),
            )
            .unwrap();
        graph
            .connect(
                auxiliary_output,
                input(&graph, auxiliary_sink),
                SendParams::default(),
            )
            .unwrap();

        let (stereo_writer, mut stereo_reader) = audio_bridge::<S>(2, 8, 4096).unwrap();
        let (auxiliary_writer, mut auxiliary_reader) = audio_bridge::<S>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, stereo_source, 2);
        constant(&mut bindings, mono_source, 1);
        bindings.bind_sink(stereo_sink, stereo_writer).unwrap();
        bindings
            .bind_sink(auxiliary_sink, auxiliary_writer)
            .unwrap();
        bindings
            .bind_rack(
                rack,
                RackSettings {
                    slots: vec![
                        RackSlot {
                            id: 90,
                            bypass: false,
                            processor: RackProcessor::Gain { initial: 2.0 },
                        },
                        RackSlot {
                            id: 12,
                            bypass: false,
                            processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                                matrix: vec![vec![0.5], vec![-1.0]],
                                ..Default::default()
                            }),
                        },
                        RackSlot {
                            id: 44,
                            bypass: false,
                            processor: RackProcessor::Gain { initial: 0.5 },
                        },
                    ],
                    routes: vec![
                        RackRoute {
                            input_port: 0,
                            output_port: 0,
                            slots: vec![90],
                        },
                        RackRoute {
                            input_port: 1,
                            output_port: 1,
                            slots: vec![12, 44],
                        },
                    ],
                },
            )
            .unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let mapper = compiled.bindings.rack_slot(rack, 12).unwrap();
        assert_ne!(compiled.bindings.rack_slot(rack, 90), Some(mapper));
        let (_, heap) = allocation::track_heap(|| compiled.engine.render(4).unwrap());
        assert_eq!(heap, (0, 0, 0));
        let mut stereo = [S::ZERO; 8];
        let mut auxiliary = [S::ZERO; 8];
        stereo_reader.read_interleaved(&mut stereo).unwrap();
        auxiliary_reader.read_interleaved(&mut auxiliary).unwrap();
        assert_samples_close(&stereo, &[0.75; 8]);
        assert_samples_close(
            &auxiliary,
            &[
                0.09375, -0.1875, 0.09375, -0.1875, 0.09375, -0.1875, 0.09375, -0.1875,
            ],
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn rack_default_route_preserves_legacy_single_chain_and_bypass_width_is_checked() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let rack = graph.create_node(NodeKind::Rack, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, rack, SendParams::default());
    connect(&mut graph, rack, sink, SendParams::default());
    let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_rack(
            rack,
            RackSettings {
                slots: vec![RackSlot {
                    id: 90,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 2.0 },
                }],
                ..Default::default()
            },
        )
        .unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    compiled.engine.render(2).unwrap();
    let mut samples = [0.0; 2];
    reader.read_interleaved(&mut samples).unwrap();
    assert_samples_close(&samples, &[0.75; 2]);

    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let rack = graph.create_rack(&[1], &[2]).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            output(&graph, source),
            graph.get_node(rack).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(rack).unwrap().outputs()[0].id(),
            input(&graph, sink),
            SendParams::default(),
        )
        .unwrap();
    let (writer, _) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_rack(
            rack,
            RackSettings {
                slots: vec![RackSlot {
                    id: 7,
                    bypass: true,
                    processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                        matrix: vec![vec![1.0], vec![0.5]],
                        ..Default::default()
                    }),
                }],
                routes: vec![RackRoute {
                    input_port: 0,
                    output_port: 0,
                    slots: vec![7],
                }],
            },
        )
        .unwrap();
    assert!(
        matches!(compile::<f64>(&graph, bindings, config()), Err(CompileError::BindingKind { node: invalid }) if invalid == rack)
    );
}

#[test]
fn rack_rejects_invalid_port_routes_and_channel_mapping_shapes() {
    for (route, matrix) in [
        (
            RackRoute {
                input_port: 4,
                output_port: 0,
                slots: vec![1],
            },
            vec![vec![1.0]],
        ),
        (
            RackRoute {
                input_port: 0,
                output_port: 4,
                slots: vec![1],
            },
            vec![vec![1.0]],
        ),
        (
            RackRoute {
                input_port: 0,
                output_port: 0,
                slots: vec![1],
            },
            vec![vec![1.0, 0.5]],
        ),
    ] {
        let mut graph = LogicalGraph::new();
        let rack = graph.create_node(NodeKind::Rack, 2).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_rack(
                rack,
                RackSettings {
                    slots: vec![RackSlot {
                        id: 1,
                        bypass: false,
                        processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                            matrix,
                            ..Default::default()
                        }),
                    }],
                    routes: vec![route],
                },
            )
            .unwrap();
        assert!(
            matches!(compile::<f64>(&graph, bindings, config()), Err(CompileError::BindingKind { node: invalid }) if invalid == rack)
        );
    }
}

#[test]
fn rack_disconnected_routed_input_and_unrouted_output_are_silent() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let rack = graph.create_node(NodeKind::Rack, 1).unwrap();
    let routed_input = graph.add_input_port(rack, 1).unwrap();
    let unrouted_output = graph.add_output_port(rack, 1).unwrap();
    let routed_sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    let silent_sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    let main_input = graph.get_node(rack).unwrap().inputs()[0].id();
    let main_output = graph.get_node(rack).unwrap().outputs()[0].id();
    graph
        .connect(output(&graph, source), main_input, SendParams::default())
        .unwrap();
    graph
        .connect(
            unrouted_output,
            input(&graph, silent_sink),
            SendParams::default(),
        )
        .unwrap();
    graph
        .connect(
            main_output,
            input(&graph, routed_sink),
            SendParams::default(),
        )
        .unwrap();
    let make_bindings = || {
        let (routed_writer, routed_reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
        let (silent_writer, silent_reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 1);
        bindings.bind_sink(routed_sink, routed_writer).unwrap();
        bindings.bind_sink(silent_sink, silent_writer).unwrap();
        bindings
            .bind_rack(
                rack,
                RackSettings {
                    routes: vec![RackRoute {
                        input_port: 1,
                        output_port: 0,
                        slots: Vec::new(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        (bindings, routed_reader, silent_reader)
    };
    let mut edge = None;
    for connected in [false, true, false] {
        if connected {
            edge = Some(
                graph
                    .connect(output(&graph, source), routed_input, SendParams::default())
                    .unwrap(),
            );
        } else if let Some(edge) = edge.take() {
            graph.disconnect(edge).unwrap();
        }
        let (bindings, mut routed_reader, mut silent_reader) = make_bindings();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let (_, heap) = allocation::track_heap(|| {
            compiled.engine.render(2).unwrap();
            compiled.engine.render(2).unwrap();
        });
        assert_eq!(heap, (0, 0, 0));
        let mut routed = [0.0; 4];
        let mut silent = [1.0; 4];
        routed_reader.read_interleaved(&mut routed).unwrap();
        silent_reader.read_interleaved(&mut silent).unwrap();
        assert_samples_close(&routed, &[if connected { 0.375 } else { 0.0 }; 4]);
        assert_samples_close(&silent, &[0.0; 4]);
    }
}
