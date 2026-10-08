use super::*;

#[test]
fn sends_preserve_fan_out_and_accept_gain_above_the_node_gain_limit() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let raw = graph.create_node(NodeKind::Sink, 2).unwrap();
    let sent = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, raw, SendParams::default());
    let edge = connect(
        &mut graph,
        source,
        sent,
        SendParams {
            gain: 32.0,
            pan: 0.5,
            ..SendParams::default()
        },
    );
    let (raw_writer, mut raw_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let (send_writer, mut send_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_sink(raw, raw_writer).unwrap();
    bindings.bind_sink(sent, send_writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let keys = compiled.bindings.edge(edge).unwrap();
    for (request_id, target, value) in [
        (1, keys.gain, ParamValue::Float(2.0)),
        (2, keys.pan, ParamValue::Float(-1.0)),
        (3, keys.mute, ParamValue::Bool(true)),
    ] {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id,
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target,
                        at: ApplyAt::Frame(2),
                        value,
                        ramp_frames: 0,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    compiled.engine.render(4).unwrap();
    let mut raw_samples = [0.0; 8];
    let mut sent_samples = [0.0; 8];
    raw_reader.read_interleaved(&mut raw_samples).unwrap();
    send_reader.read_interleaved(&mut sent_samples).unwrap();
    assert_eq!(raw_samples, [0.375; 8]);
    assert_eq!(sent_samples, [6.0, 12.0, 6.0, 12.0, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn edge_gain_and_pan_ramps_cross_blocks_and_nonstereo_pan_updates_are_rejected() {
    for channels in [1, 2, 4] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        let edge = connect(&mut graph, source, sink, SendParams::default());
        let (writer, mut reader) = audio_bridge::<f64>(channels, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, channels);
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let keys = compiled.bindings.edge(edge).unwrap();
        let request = ParameterRequest {
            request_id: 1,
            plan_revision: 7,
            timeline_epoch: 3,
            target: keys.pan,
            at: ApplyAt::Frame(1),
            value: ParamValue::Float(1.0),
            ramp_frames: 4,
        };
        assert_eq!(
            compiled.control.submit(request, 0).code,
            if channels == 2 {
                ReplyCode::Accepted
            } else {
                ReplyCode::InvalidValue
            }
        );
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: 2,
                        target: keys.gain,
                        value: ParamValue::Float(0.0),
                        ..request
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
        for frames in [2, 1, 3] {
            compiled.engine.render(frames).unwrap();
        }
        let mut samples = vec![0.0; channels * 6];
        reader.read_interleaved(&mut samples).unwrap();
        let expected = (0..6)
            .flat_map(|frame| {
                let progress = if frame == 0 {
                    0.0
                } else {
                    (frame as f64 / 4.0).min(1.0)
                };
                (0..channels).map(move |channel| {
                    let balance = if channels == 2 && channel == 0 {
                        1.0 - progress
                    } else {
                        1.0
                    };
                    0.375 * (1.0 - progress) * balance
                })
            })
            .collect::<Vec<_>>();
        assert_samples(&samples, &expected);
    }
}

#[test]
fn send_attenuation_precedes_gain_overflow() {
    for (pan, expected_left) in [(1.0, 0.0), (0.5, f64::MAX)] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        connect(
            &mut graph,
            source,
            sink,
            SendParams {
                gain: f64::MAX,
                pan,
                ..SendParams::default()
            },
        );
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
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        compiled.engine.render(1).unwrap();
        let mut samples = [0.0; 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples[0], expected_left);
        assert_eq!(samples[1], f64::INFINITY);
    }
}
