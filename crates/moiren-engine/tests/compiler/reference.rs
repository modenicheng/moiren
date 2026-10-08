use super::*;

fn random_dags<S: ProcessingSample>() {
    let mut rng = 0x73b7_9816_u64;
    let mut next = || {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (rng >> 32) as usize
    };
    for _ in 0..24 {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let mut nodes = vec![source];
        let mut expected = vec![[0.375, 0.375]];
        let mut bindings = NodeBindings::<S>::new();
        constant(&mut bindings, source, 2);
        for _ in 0..10 {
            let kind = [NodeKind::Gain, NodeKind::Pan, NodeKind::Bus][next() % 3];
            let node = graph.create_node(kind, 2).unwrap();
            let mut value = [0.0, 0.0];
            let ports = if kind == NodeKind::Bus {
                (0..next() % 4)
                    .map(|_| graph.add_input_port(node, 2).unwrap())
                    .collect::<Vec<_>>()
            } else {
                vec![input(&graph, node)]
            };
            for port in ports {
                if next() % 5 == 0 {
                    continue;
                }
                let index = next() % nodes.len();
                let params = SendParams {
                    gain: [0.0, 0.5, 1.0, 2.0][next() % 4],
                    pan: [-1.0, -0.5, 0.0, 0.5, 1.0][next() % 5],
                    mute: next() % 7 == 0,
                    ..SendParams::default()
                };
                graph
                    .connect(output(&graph, nodes[index]), port, params)
                    .unwrap();
                if !params.mute {
                    // Independent sample evaluator; no compiler IR/DSP helpers.
                    value[0] += expected[index][0] * params.gain * (1.0 - params.pan.max(0.0));
                    value[1] += expected[index][1] * params.gain * (1.0 + params.pan.min(0.0));
                }
            }
            if kind == NodeKind::Gain {
                let gain = [0.25, 0.5, 1.0, 2.0][next() % 4];
                bindings.bind_gain(node, gain).unwrap();
                value[0] *= gain;
                value[1] *= gain;
            } else if kind == NodeKind::Pan {
                let pan: f64 = [-1.0, -0.5, 0.0, 0.5, 1.0][next() % 5];
                bindings.bind_pan(node, pan).unwrap();
                value[0] *= 1.0 - pan.max(0.0);
                value[1] *= 1.0 + pan.min(0.0);
            }
            nodes.push(node);
            expected.push(value);
        }
        let mut sinks = Vec::new();
        for index in [0, next() % nodes.len(), nodes.len() - 1] {
            let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
            connect(&mut graph, nodes[index], sink, SendParams::default());
            let (writer, reader) = audio_bridge::<S>(2, 16, 4096).unwrap();
            bindings.bind_sink(sink, writer).unwrap();
            sinks.push((reader, expected[index]));
        }
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        for frames in [1, 8, 3, 2] {
            compiled.engine.render(frames).unwrap();
            for (reader, value) in &mut sinks {
                let mut samples = vec![S::ZERO; frames * 2];
                assert_eq!(
                    reader
                        .read_interleaved(&mut samples)
                        .unwrap()
                        .transferred_frames,
                    frames
                );
                let reference = (0..frames).flat_map(|_| *value).collect::<Vec<_>>();
                assert_samples(&samples, &reference);
            }
        }
    }
}

#[test]
fn random_dags_match_independent_sample_evaluation_for_both_precisions() {
    random_dags::<f32>();
    random_dags::<f64>();
}
