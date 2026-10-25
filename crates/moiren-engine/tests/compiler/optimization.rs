use super::*;

#[test]
fn linear_gain_pan_chain_reuses_dead_inputs_in_place() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain_a = graph.create_node(NodeKind::Gain, 2).unwrap();
    let pan = graph.create_node(NodeKind::Pan, 2).unwrap();
    let gain_b = graph.create_node(NodeKind::Gain, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, gain_a, SendParams::default());
    connect(&mut graph, gain_a, pan, SendParams::default());
    connect(&mut graph, pan, gain_b, SendParams::default());
    connect(&mut graph, gain_b, sink, SendParams::default());

    let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_gain(gain_a, 2.0).unwrap();
    bindings.bind_pan(pan, 0.5).unwrap();
    bindings.bind_gain(gain_b, 0.5).unwrap();
    bindings.bind_sink(sink, writer).unwrap();

    let mut compiled = compile(&graph, bindings, config()).unwrap();
    assert_eq!(compiled.stats.fused_operation_count, 0);
    assert_eq!(compiled.stats.in_place_count, 3);
    assert_eq!(compiled.stats.slot_count, 2);
    assert_eq!(compiled.stats.operation_count, 9);

    compiled.engine.render(4).unwrap();
    let mut samples = [0.0; 8];
    reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(
        samples,
        [0.1875, 0.375, 0.1875, 0.375, 0.1875, 0.375, 0.1875, 0.375]
    );
}

#[test]
fn bus_send_parameters_are_lowered_into_one_mix_operation() {
    let mut graph = LogicalGraph::new();
    let first = graph.create_node(NodeKind::Source, 2).unwrap();
    let second = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let first_input = graph.add_input_port(bus, 2).unwrap();
    let second_input = graph.add_input_port(bus, 2).unwrap();
    graph
        .connect(
            output(&graph, first),
            first_input,
            SendParams {
                gain: 0.5,
                ..SendParams::default()
            },
        )
        .unwrap();
    graph
        .connect(
            output(&graph, second),
            second_input,
            SendParams {
                gain: 2.0,
                pan: 0.5,
                ..SendParams::default()
            },
        )
        .unwrap();
    connect(&mut graph, bus, sink, SendParams::default());

    let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, first, 2);
    constant(&mut bindings, second, 2);
    bindings.bind_sink(sink, writer).unwrap();

    let mut compiled = compile(&graph, bindings, config()).unwrap();
    assert_eq!(compiled.stats.mix_operation_count, 1);
    assert_eq!(compiled.stats.operation_count, 5);
    assert_eq!(compiled.stats.slot_count, 3);

    compiled.engine.render(4).unwrap();
    let mut samples = [0.0; 8];
    reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(
        samples,
        [
            0.5625, 0.9375, 0.5625, 0.9375, 0.5625, 0.9375, 0.5625, 0.9375
        ]
    );
}

#[test]
fn separate_operations_reuse_dead_slots_within_the_audio_budget() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let first = graph.create_node(NodeKind::Delay, 2).unwrap();
    let second = graph.create_node(NodeKind::Delay, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, first, SendParams::default());
    connect(&mut graph, first, second, SendParams::default());
    connect(&mut graph, second, sink, SendParams::default());
    let bind = || {
        let (writer, reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 2);
        for node in [first, second] {
            bindings
                .bind_delay(
                    node,
                    DelaySettings {
                        delay_ms: 0.0,
                        max_delay_ms: 1.0,
                        mix: 0.0,
                        ..DelaySettings::default()
                    },
                )
                .unwrap();
        }
        bindings.bind_sink(sink, writer).unwrap();
        (bindings, reader)
    };
    let (bindings, _) = bind();
    let reference = compile_reference(&graph, bindings, config()).unwrap();
    assert_eq!(reference.stats.slot_count, 6);
    let (bindings, mut reader) = bind();
    let mut cfg = config();
    cfg.audio_byte_budget = 2 * 2 * 8 * size_of::<f64>();
    let mut optimized = compile(&graph, bindings, cfg).unwrap();
    assert_eq!(optimized.stats.in_place_count, 0);
    assert_eq!(optimized.stats.slot_count, 2);
    assert_eq!(
        optimized.stats.operation_count,
        reference.stats.operation_count
    );
    for frames in [1, 8, 3, 2] {
        let (report, counts) = allocation::track_allocations(|| optimized.engine.render(frames));
        report.unwrap();
        assert_eq!(counts, (0, 0));
        let mut samples = vec![0.0; frames * 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples, vec![0.375; frames * 2]);
    }
}

#[test]
fn fused_mix_preserves_attenuation_before_overflow() {
    for pan in [1.0, 0.5] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let port = graph.add_input_port(bus, 2).unwrap();
        graph
            .connect(
                output(&graph, source),
                port,
                SendParams {
                    gain: f64::MAX,
                    pan,
                    ..SendParams::default()
                },
            )
            .unwrap();
        connect(&mut graph, bus, sink, SendParams::default());
        let mut results = Vec::new();
        for optimized in [false, true] {
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
            let mut compiled = if optimized {
                compile(&graph, bindings, config())
            } else {
                compile_reference(&graph, bindings, config())
            }
            .unwrap();
            compiled.engine.render(1).unwrap();
            let mut samples = [0.0; 2];
            reader.read_interleaved(&mut samples).unwrap();
            results.push(samples.map(f64::to_bits));
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn fused_mix_preserves_f32_rounding_after_every_addition() {
    let mut graph = LogicalGraph::new();
    let large = graph.create_node(NodeKind::Source, 1).unwrap();
    let small = graph.create_node(NodeKind::Source, 1).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    for source in [large, small, small] {
        let port = graph.add_input_port(bus, 1).unwrap();
        graph
            .connect(output(&graph, source), port, SendParams::default())
            .unwrap();
    }
    connect(&mut graph, bus, sink, SendParams::default());
    let mut results = Vec::new();
    for optimized in [false, true] {
        let (writer, mut reader) = audio_bridge::<f32>(1, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        for (source, value) in [(large, 16_777_216.0), (small, 1.0)] {
            bindings
                .bind_source(source, ConstantSource { channels: 1, value })
                .unwrap();
        }
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = if optimized {
            compile(&graph, bindings, config())
        } else {
            compile_reference(&graph, bindings, config())
        }
        .unwrap();
        compiled.engine.render(1).unwrap();
        let mut samples = [0.0; 1];
        reader.read_interleaved(&mut samples).unwrap();
        results.push(samples[0].to_bits());
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn mono_and_multichannel_mix_ramps_match_reference_across_tiles() {
    fn compare<S: ProcessingSample>(channels: usize) {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let bus = graph.create_node(NodeKind::Bus, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        let mut edges = Vec::new();
        for gain in [0.3, 0.71] {
            let port = graph.add_input_port(bus, channels).unwrap();
            edges.push(
                graph
                    .connect(
                        output(&graph, source),
                        port,
                        SendParams {
                            gain,
                            ..SendParams::default()
                        },
                    )
                    .unwrap(),
            );
        }
        connect(&mut graph, bus, sink, SendParams::default());
        let mut results = Vec::new();
        for optimized in [false, true] {
            let (writer, mut reader) = audio_bridge::<S>(channels, 128, 8192).unwrap();
            let mut bindings = NodeBindings::new();
            bindings
                .bind_source(
                    source,
                    ConstantSource {
                        channels,
                        value: 0.123,
                    },
                )
                .unwrap();
            bindings.bind_sink(sink, writer).unwrap();
            let mut cfg = config();
            cfg.engine.max_block_frames = 96;
            let mut compiled = if optimized {
                compile(&graph, bindings, cfg)
            } else {
                compile_reference(&graph, bindings, cfg)
            }
            .unwrap();
            assert_eq!(
                compiled
                    .control
                    .submit(
                        ParameterRequest {
                            request_id: 1,
                            plan_revision: 7,
                            timeline_epoch: 3,
                            target: compiled.bindings.edge(edges[0]).unwrap().gain,
                            at: ApplyAt::Frame(3),
                            value: ParamValue::Float(1.7),
                            ramp_frames: 113,
                        },
                        0
                    )
                    .code,
                ReplyCode::Accepted
            );
            let mut collected = Vec::new();
            for frames in [65, 96, 17] {
                let (report, counts) = allocation::track_heap(|| compiled.engine.render(frames));
                report.unwrap();
                assert_eq!(counts, (0, 0, 0));
                let mut samples = vec![S::ZERO; channels * frames];
                assert_eq!(
                    reader
                        .read_interleaved(&mut samples)
                        .unwrap()
                        .transferred_frames,
                    frames
                );
                collected.extend(samples.into_iter().map(|sample| sample.to_f64().to_bits()));
            }
            results.push(collected);
        }
        assert_eq!(results[0], results[1]);
    }
    for channels in [1, 4] {
        compare::<f32>(channels);
        compare::<f64>(channels);
    }
}

#[test]
fn empty_fused_bus_retains_parameter_table_identity() {
    let mut graph = LogicalGraph::new();
    graph.create_node(NodeKind::Bus, 2).unwrap();
    let compiled = compile::<f64>(&graph, NodeBindings::new(), config()).unwrap();
    let parts = compiled.engine.into_parts();
    let (_, foreign) =
        moiren_engine::control::parameter_channel(Vec::new(), 7, 3, 16, 48_000).unwrap();
    assert!(matches!(
        Engine::new(parts.plan, parts.resources, foreign),
        Err(RuntimeError::ForeignParameters)
    ));
}
