use super::*;
use moiren_engine::{buffer::*, signal::ProcessSignals};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct TimelineSource;
impl<S: ProcessingSample> RtAudioSource<S> for TimelineSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        for (channel, samples) in output.channels_mut().enumerate() {
            for (frame, sample) in samples.iter_mut().enumerate() {
                *sample = S::from_f64(
                    ((ctx.timeline_start + frame as u64) as f64 * 0.07 - 0.33)
                        * (channel + 1) as f64,
                );
            }
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

fn automated_mix<S: ProcessingSample>(optimized: bool) -> (CompileStats, Vec<Vec<u64>>) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let pan = graph.create_node(NodeKind::Pan, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let mixed = graph.create_node(NodeKind::Sink, 2).unwrap();
    let raw = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(
        &mut graph,
        source,
        gain,
        SendParams {
            gain: 0.7,
            pan: 0.13,
            ..SendParams::default()
        },
    );
    connect(&mut graph, gain, pan, SendParams::default());
    let ports: Vec<_> = (0..6)
        .map(|_| graph.add_input_port(bus, 2).unwrap())
        .collect();
    let mut edges = Vec::new();
    for (port, node, params) in [
        (
            ports[1],
            gain,
            SendParams {
                tap: SendTap::PreFader,
                gain: 0.3,
                ..SendParams::default()
            },
        ),
        (
            ports[2],
            pan,
            SendParams {
                gain: 1.7,
                pan: -0.31,
                ..SendParams::default()
            },
        ),
        (
            ports[3],
            source,
            SendParams {
                gain: 0.6,
                pan: 0.11,
                ..SendParams::default()
            },
        ),
        (
            ports[4],
            source,
            SendParams {
                gain: 0.2,
                mute: true,
                ..SendParams::default()
            },
        ),
    ] {
        edges.push(graph.connect(output(&graph, node), port, params).unwrap());
    }
    connect(&mut graph, bus, mixed, SendParams::default());
    connect(&mut graph, source, raw, SendParams::default());
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, TimelineSource).unwrap();
    bindings.bind_gain(gain, 1.3).unwrap();
    bindings
        .bind_pan_with_stereo(pan, -0.27, true, true)
        .unwrap();
    let mut readers = Vec::new();
    for node in [mixed, raw] {
        let (writer, reader) = audio_bridge::<S>(2, 128, 4096).unwrap();
        bindings.bind_sink(node, writer).unwrap();
        readers.push(reader);
    }
    let mut cfg = config();
    cfg.engine.max_block_frames = 96;
    let mut compiled = if optimized {
        compile(&graph, bindings, cfg)
    } else {
        compile_reference(&graph, bindings, cfg)
    }
    .unwrap();
    let keys: Vec<_> = edges
        .iter()
        .map(|edge| compiled.bindings.edge(*edge).unwrap())
        .collect();
    let node_gain = ParameterKey {
        processor: compiled.bindings.node(gain).unwrap(),
        parameter: Gain::LEVEL,
    };
    let node_pan = ParameterKey {
        processor: compiled.bindings.node(pan).unwrap(),
        parameter: Pan::POSITION,
    };
    for (index, (at, target, value, ramp_frames)) in [
        (0, keys[0].gain, ParamValue::Float(2.3), 113),
        (0, keys[0].pan, ParamValue::Float(0.73), 110),
        (0, node_gain, ParamValue::Float(0.8), 109),
        (0, node_pan, ParamValue::Float(0.61), 173),
        (2, keys[3].mute, ParamValue::Bool(false), 0),
        (2, keys[3].gain, ParamValue::Float(3.0), 107),
        (4, keys[2].pan, ParamValue::Float(-0.23), 108),
        (4, keys[2].gain, ParamValue::Float(1.53), 109),
        (5, keys[0].mute, ParamValue::Bool(true), 0),
        (7, keys[0].mute, ParamValue::Bool(false), 0),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: index as u64,
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target,
                        at: ApplyAt::Frame(at),
                        value,
                        ramp_frames,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    let mut output = vec![Vec::new(); readers.len()];
    for frames in [1, 3, 2, 8, 1, 4, 32, 65, 96, 17] {
        let (report, counts) = allocation::track_heap(|| compiled.engine.render(frames));
        report.unwrap();
        assert_eq!(counts, (0, 0, 0));
        for (reader, collected) in readers.iter_mut().zip(&mut output) {
            let mut samples = vec![S::ZERO; frames * 2];
            assert_eq!(
                reader
                    .read_interleaved(&mut samples)
                    .unwrap()
                    .transferred_frames,
                frames
            );
            collected.extend(samples.into_iter().map(|sample| sample.to_f64().to_bits()));
        }
    }
    (compiled.stats, output)
}

#[test]
fn fused_mix_matches_reference_bit_for_bit_with_ramps_prefader_fanout_and_repeated_sources() {
    fn compare<S: ProcessingSample>() {
        let (reference, expected) = automated_mix::<S>(false);
        let (optimized, actual) = automated_mix::<S>(true);
        assert_eq!(actual, expected);
        assert_eq!(optimized.mix_operation_count, 1);
        assert_eq!(optimized.operation_count + 4, reference.operation_count);
        assert!(optimized.slot_count < reference.slot_count);
        assert!(optimized.audio_bytes < reference.audio_bytes);
    }
    compare::<f32>();
    compare::<f64>();
}

struct SampleObserver(Arc<AtomicU64>);
impl RtObserver<f64> for SampleObserver {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0) == Some(1) {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn observe(&mut self, _: &ProcessContext, inputs: ReadPorts<'_, f64>, _: ProcessSignals<'_>) {
        self.0.store(
            inputs.get(0).unwrap().channel(0)[0].to_bits(),
            Ordering::Relaxed,
        );
    }
}

#[test]
fn observing_bus_input_keeps_post_send_samples_and_disconnected_silence() {
    for connected in [false, true] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let bus = graph.create_node(NodeKind::Bus, 1).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
        let port = graph.add_input_port(bus, 1).unwrap();
        if connected {
            graph
                .connect(
                    output(&graph, source),
                    port,
                    SendParams {
                        gain: 0.5,
                        ..SendParams::default()
                    },
                )
                .unwrap();
        }
        connect(&mut graph, bus, sink, SendParams::default());
        let observed = Arc::new(AtomicU64::new(f64::NAN.to_bits()));
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 1);
        bindings
            .observe_input(port, SampleObserver(observed.clone()))
            .unwrap();
        let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        assert_eq!(compiled.stats.mix_operation_count, 0);
        for frames in [8, 1, 3] {
            let (report, counts) = allocation::track_allocations(|| compiled.engine.render(frames));
            report.unwrap();
            assert_eq!(counts, (0, 0));
            let expected = if connected { 0.1875 } else { 0.0 };
            assert_eq!(f64::from_bits(observed.load(Ordering::Relaxed)), expected);
            let mut samples = vec![f64::NAN; frames];
            reader.read_interleaved(&mut samples).unwrap();
            assert_eq!(samples, vec![expected; frames]);
        }
    }
}
