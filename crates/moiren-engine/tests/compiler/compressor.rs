use super::*;

fn graph() -> (LogicalGraph, NodeId, NodeId, NodeId) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 4).unwrap();
    let compressor = graph.create_node(NodeKind::Compressor, 4).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 4).unwrap();
    connect(&mut graph, source, compressor, SendParams::default());
    connect(&mut graph, compressor, sink, SendParams::default());
    (graph, source, compressor, sink)
}

fn automated<S: ProcessingSample>() {
    let (graph, source, compressor, sink) = graph();
    let (writer, mut reader) = audio_bridge::<S>(4, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 4,
                value: 1.0,
            },
        )
        .unwrap();
    bindings
        .bind_compressor(
            compressor,
            CompressorSettings {
                threshold_db: -12.0,
                ratio: 4.0,
                attack_ms: 0.0,
                release_ms: 0.0,
                knee_db: 0.0,
                ..CompressorSettings::default()
            },
        )
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let processor = compiled.bindings.node(compressor).unwrap();
    for (parameter, at, value, ramp_frames) in [
        (Compressor::THRESHOLD, 0, 0.0, 4),
        (Compressor::MIX, 4, 0.0, 0),
        (Compressor::OUTPUT_GAIN, 4, -6.0, 4),
    ] {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: at + u64::from(parameter.0),
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target: ParameterKey {
                            processor,
                            parameter
                        },
                        at: ApplyAt::Frame(at),
                        value: ParamValue::Float(value),
                        ramp_frames
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    let (results, allocations) = super::allocation::track_allocations(|| {
        [1, 2, 2, 3].map(|frames| compiled.engine.render(frames))
    });
    for result in results {
        result.unwrap();
    }
    assert_eq!(allocations, (0, 0));
    let mut samples = [S::ZERO; 32];
    reader.read_interleaved(&mut samples).unwrap();
    for (frame, gain_db) in [-6.75, -4.5, -2.25, 0.0, -1.5, -3.0, -4.5, -6.0]
        .into_iter()
        .enumerate()
    {
        assert_samples(
            &samples[frame * 4..(frame + 1) * 4],
            &[10.0_f64.powf(gain_db / 20.0); 4],
        );
    }
    for _ in 0..3 {
        assert_eq!(
            compiled.control.poll_applied().unwrap().code,
            ReplyCode::Applied
        );
    }
    assert_eq!(
        compiled
            .control
            .submit(
                ParameterRequest {
                    request_id: 99,
                    plan_revision: 7,
                    timeline_epoch: 3,
                    target: ParameterKey {
                        processor,
                        parameter: Compressor::RATIO
                    },
                    at: ApplyAt::NextBlock,
                    value: ParamValue::Float(0.5),
                    ramp_frames: 0
                },
                8
            )
            .code,
        ReplyCode::InvalidValue
    );
}

#[test]
fn compressor_compilation_and_sample_automation_work_in_both_precisions() {
    automated::<f32>();
    automated::<f64>();
}

#[test]
fn compressor_default_compiles_and_binding_errors_are_control_side() {
    let (graph, source, compressor, sink) = graph();
    let (writer, mut reader) = audio_bridge::<f64>(4, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 4,
                value: 0.0,
            },
        )
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    compiled.engine.render(8).unwrap();
    let mut samples = [1.0; 32];
    reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.0; 32]);
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_compressor(compressor, CompressorSettings::default())
        .unwrap();
    assert_eq!(
        bindings.bind_compressor(compressor, CompressorSettings::default()),
        Err(CompileError::DuplicateBinding { node: compressor })
    );
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_compressor(source, CompressorSettings::default())
        .unwrap();
    assert!(
        matches!(compile(&graph, bindings, config()), Err(CompileError::BindingKind { node }) if node == source)
    );
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 4,
                value: 0.0,
            },
        )
        .unwrap();
    let (writer, _) = audio_bridge::<f64>(4, 8, 4096).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_compressor(
            compressor,
            CompressorSettings {
                ratio: 0.0,
                ..CompressorSettings::default()
            },
        )
        .unwrap();
    assert!(matches!(
        compile(&graph, bindings, config()),
        Err(CompileError::Control(ControlError::InvalidConfiguration))
    ));
}
