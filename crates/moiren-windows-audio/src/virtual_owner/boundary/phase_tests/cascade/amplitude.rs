//! Host-only amplitude and PCM16 controls, never native phase attribution.
use super::*;

// A host-only amplitude control. The oracle shares production FIR arithmetic
// and observed correction, not Bus samples, native clocks or a fitted phase.
fn encoded_amplitude_case<S: ProcessingSample>(dc: bool) {
    use crate::{
        latency::MarkerGenerator,
        vad::{Channels, SampleFormat, encode_interleaved},
    };

    const BLOCK: usize = 64;
    const CALLBACKS: usize = 256;
    const AMPLITUDE: f32 = 0.025;
    let mut signal = vec![[if dc { AMPLITUDE } else { 0.0 }; 2]; BLOCK * CALLBACKS + 1024];
    if !dc {
        let mut markers = vec![0.0; 4 * 1024];
        MarkerGenerator::new(markers.len(), AMPLITUDE)
            .unwrap()
            .fill(&mut markers);
        for (index, marker) in markers.as_chunks::<1024>().0.iter().enumerate() {
            for (offset, &value) in marker.iter().enumerate() {
                signal[1024 + index * 4096 + offset] = [value; 2];
            }
        }
    }
    let mut graph = LogicalGraph::new();
    let source_node = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let sink_node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let input = graph.add_input_port(bus, 2).unwrap();
    graph
        .connect(
            graph.get_node(source_node).unwrap().outputs()[0].id(),
            input,
            Default::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(sink_node).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    assert_eq!(graph.nodes().len(), 3);
    assert_eq!(graph.edges().len(), 2);
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let (writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let source = VirtualPlaybackSource::new(reader, 2, BLOCK, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(playback.clone());
    let sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(microphone.clone());
    let captured = Arc::new(Mutex::new(Captured::default()));
    let mut bindings = NodeBindings::<S>::new();
    bindings
        .bind_source(source_node, SourceCheck(source, captured.clone()))
        .unwrap();
    bindings
        .bind_sink(sink_node, SinkCheck(sink, captured.clone()))
        .unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: BLOCK,
                max_events_per_block: 8,
            },
            audio_byte_budget: BUDGET,
            plan_revision: 13,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    assert_eq!(compiled.stats.mix_operation_count, 1);
    let mut playback_oracle = StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let mut microphone_oracle = StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let (mut submitted, mut consumed) = (0, 0);
    let (mut actual_pcm, mut expected_pcm) = (Vec::new(), Vec::new());
    for index in 0..CALLBACKS {
        let target: usize = if index < 128 { 768 } else { 384 };
        let queued = producer.capacity_frames() - producer.available_frames();
        let fill = target.saturating_sub(queued);
        let input: Vec<_> = signal[submitted..submitted + fill]
            .iter()
            .flatten()
            .copied()
            .collect();
        assert!(producer.write_interleaved_exact(&input).unwrap());
        submitted += fill;
        compiled.engine.render(BLOCK).unwrap();
        let source = playback.src_checkpoint().unwrap();
        let sink = microphone.src_checkpoint().unwrap();
        for point in [source, sink] {
            assert_eq!(point.xruns, 0);
            assert_eq!(point.transferred_frames, BLOCK as u64);
        }
        let capture = captured.lock().unwrap();
        playback_oracle.set_correction_ppm(capture.playback_ppm);
        let pushed =
            (source.after.unwrap().input_frames - source.before.unwrap().input_frames) as usize;
        assert!(consumed + pushed <= submitted);
        for frame in &signal[consumed..consumed + pushed] {
            assert!(playback_oracle.push_frame(frame.map(f64::from)));
        }
        consumed += pushed;
        microphone_oracle.set_correction_ppm(capture.microphone_ppm);
        for (offset, captured_frame) in capture.bus.iter().enumerate().take(BLOCK) {
            let frame = playback_oracle.next_frame().unwrap();
            let expected = frame.map(|value| {
                if offset < source.muted_frames as usize {
                    0.0
                } else {
                    S::from_f64(value).to_f64()
                }
            });
            assert_eq!(*captured_frame, expected, "unity Bus changed amplitude");
            assert!(microphone_oracle.push_frame(expected));
        }
        assert_eq!(playback_oracle.cursor(), source.after.unwrap());
        let produced =
            (sink.after.unwrap().output_frames - sink.before.unwrap().output_frames) as usize;
        let mut actual = vec![0.0; produced * 2];
        let read = consumer.read_interleaved(&mut actual).unwrap();
        assert_eq!(read.transferred_frames, produced);
        for frame in actual.as_chunks::<2>().0 {
            let expected = microphone_oracle
                .next_frame()
                .unwrap()
                .map(|value| value as f32);
            assert_eq!(*frame, expected, "MIC differs from independent double FIR");
            assert_eq!(frame[0], frame[1], "stereo channels must not be summed");
            expected_pcm.extend(expected);
        }
        assert_eq!(microphone_oracle.cursor(), sink.after.unwrap());
        actual_pcm.extend(actual);
    }
    assert!(actual_pcm.len() > BLOCK * CALLBACKS);
    let mut encoded = vec![0; actual_pcm.len() * 2];
    encode_interleaved(
        &actual_pcm,
        &mut encoded,
        SampleFormat::Pcm16,
        Channels::Stereo,
    )
    .unwrap();
    for (bytes, &expected) in encoded.as_chunks::<2>().0.iter().zip(&expected_pcm) {
        // Scalar PCM16 contract is independent of the encoder under test.
        let quantized = (expected.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        assert_eq!(i16::from_le_bytes(*bytes), quantized);
    }
    if dc {
        assert!(
            actual_pcm[actual_pcm.len() - 4096..]
                .iter()
                .all(|&value| (value - AMPLITUDE).abs() < 1e-7)
        );
        assert!(
            encoded[encoded.len() - 8192..]
                .as_chunks::<2>()
                .0
                .iter()
                .all(|bytes| i16::from_le_bytes(*bytes) == 819)
        );
    } else {
        assert!(actual_pcm.iter().any(|&value| value > AMPLITUDE));
        assert!(actual_pcm.iter().any(|&value| value < -AMPLITUDE));
    }
}

#[test]
fn src_cascade_engine64_sparse_markers_and_dc_match_unity_bus_firs_and_pcm16() {
    for dc in [false, true] {
        encoded_amplitude_case::<f32>(dc);
        encoded_amplitude_case::<f64>(dc);
    }
}
