use super::*;

#[test]
fn playback_zero_refill_segment_does_not_rearm_prime_with_valid_fir_history() {
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    producer.write_interleaved_exact(&pcm(384)).unwrap();
    let mut source = VirtualPlaybackSource::new(reader, 2, 32, 96_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap();
    let mut buffer = CallbackBuffer::<f32>::new(1, 96_000);
    for frame in 0..64 {
        buffer.context.timeline_start = frame;
        let report = buffer.read_source(&mut source);
        assert_eq!(report.transferred_frames, 1, "Engine frame={frame}");
        assert!(!report.discontinuity, "Engine frame={frame}");
        assert_eq!(report.xruns, 0, "Engine frame={frame}");
    }
}

fn segment_matrix<S: ProcessingSample>() {
    for rate in [44_100, 48_000, 88_200, 96_000] {
        for explicit in [false, true] {
            for frames in [1, 2, 3, 7, 16, 32] {
                let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
                let source = VirtualPlaybackSource::new(reader, 2, 32, rate, BUDGET).unwrap();
                let probe = Arc::new(BoundaryProbe::default());
                let mut source = if explicit {
                    source.with_clock_drift_target(384).unwrap()
                } else {
                    source.with_clock_drift().unwrap()
                }
                .with_probe(probe.clone());
                let target = probe.snapshot().target_frames.unwrap() as usize;
                let content = pcm(target);
                producer.write_interleaved_exact(&content).unwrap();
                let mut buffer = CallbackBuffer::<S>::new(frames, rate);
                let mut muted = SRC_HEADROOM_FRAMES;
                for callback in 0..128 {
                    buffer.context.timeline_start = (callback * frames) as u64;
                    let before = source.carrier.available_frames();
                    let report = crate::test_allocation::assert_allocates_nothing(|| {
                        buffer.read_source(&mut source)
                    });
                    assert_eq!(
                        report.transferred_frames, frames,
                        "rate={rate} frames={frames}"
                    );
                    assert!(!report.discontinuity);
                    assert_eq!(report.xruns, 0);
                    buffer.assert_pcm(muted.min(frames));
                    muted = muted.saturating_sub(frames);
                    let consumed = before - source.carrier.available_frames();
                    producer
                        .write_interleaved_exact(&content[..consumed * 2])
                        .unwrap();
                }
                let events = probe.snapshot().playback_events.unwrap();
                assert_eq!(events.initial_prime_muted_frames, 32);
                assert_eq!(events.recovery_prime_muted_frames, 0);
                assert_eq!(events.short_read_callbacks, 0);
                assert!(events.initial_prime_complete);
                assert_eq!(probe.snapshot().xruns, 0);
            }
        }
    }
}

#[test]
fn playback_short_segment_rate_target_and_precision_matrix_remains_primed() {
    segment_matrix::<f32>();
    segment_matrix::<f64>();
}

#[test]
fn playback_empty_carrier_zero_refill_uses_history_then_real_starvation_rearms_prime() {
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    producer.write_interleaved_exact(&pcm(384)).unwrap();
    let probe = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, 32, 96_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(probe.clone());
    let mut buffer = CallbackBuffer::<f64>::new(1, 96_000);
    let mut reached_empty = false;
    for _ in 0..1024 {
        let report = buffer.read_source(&mut source);
        assert_eq!(report.transferred_frames, 1);
        assert_eq!(report.xruns, 0);
        if source.carrier.available_frames() == 0 {
            reached_empty = true;
            break;
        }
    }
    assert!(reached_empty);
    assert_eq!(
        source.resampler.as_ref().unwrap().input_frames_needed(1),
        Some(0)
    );
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 1);
    buffer.assert_pcm(0);
    let point = probe.src_checkpoint().unwrap();
    assert_eq!(point.carrier_frames, 0);
    assert_eq!(
        point.after.unwrap().output_frames,
        point.before.unwrap().output_frames + 1
    );
    assert_eq!(
        probe
            .snapshot()
            .playback_events
            .unwrap()
            .short_read_callbacks,
        0
    );
    let mut starved = false;
    for _ in 0..4 {
        let report = buffer.read_source(&mut source);
        if report.transferred_frames == 0 {
            assert!(report.discontinuity);
            assert_eq!(report.xruns, 1);
            starved = true;
            break;
        }
    }
    assert!(starved);
    producer.write_interleaved_exact(&pcm(383)).unwrap();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
    producer.write_interleaved_exact(&pcm(1)).unwrap();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 1);
    buffer.assert_pcm(1);
    let events = probe.snapshot().playback_events.unwrap();
    assert_eq!(events.short_read_callbacks, 1);
    assert_eq!(events.recovery_prime_muted_frames, 1);
}

#[test]
fn playback_zero_refill_cannot_bypass_epoch_fencing() {
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    producer.write_interleaved_exact(&pcm(384)).unwrap();
    let reset = Arc::new(AtomicU64::new(0));
    let producer_ack = Arc::new(AtomicU64::new(0));
    let consumer_ack = Arc::new(AtomicU64::new(0));
    let mut source = VirtualPlaybackSource::new(reader, 2, 32, 96_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_reset_fence(reset.clone(), producer_ack.clone(), consumer_ack.clone());
    let mut buffer = CallbackBuffer::<f32>::new(1, 96_000);
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 1);
    assert_eq!(
        source.resampler.as_ref().unwrap().input_frames_needed(1),
        Some(0)
    );
    reset.store(1, Ordering::Release);
    let pending = source.carrier.available_frames();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
    assert_eq!(source.carrier.available_frames(), pending);
    assert_eq!(consumer_ack.load(Ordering::Acquire), 0);
    producer_ack.store(1, Ordering::Release);
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
    assert_eq!(consumer_ack.load(Ordering::Acquire), 1);
    assert_eq!(source.carrier.available_frames(), 0);
    producer.write_interleaved_exact(&pcm(384)).unwrap();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 1);
    buffer.assert_pcm(1);
}

fn compiled_segments<S: ProcessingSample>() {
    use moiren_core::{
        graph::{LogicalGraph, NodeKind},
        protocol::{ApplyAt, ParamValue, ParameterRequest, ReplyCode},
    };
    use moiren_engine::{
        compiler::{CompileConfig, NodeBindings, compile},
        runtime::EngineConfig,
    };

    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let output = graph.create_node(NodeKind::Sink, 2).unwrap();
    let bus_input = graph.add_input_port(bus, 2).unwrap();
    let edge = graph
        .connect(
            graph.get_node(input).unwrap().outputs()[0].id(),
            bus_input,
            Default::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(output).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let source_probe = Arc::new(BoundaryProbe::default());
    let source = VirtualPlaybackSource::new(reader, 2, 32, 96_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(source_probe.clone());
    // The sink is a bounded Engine-rate test carrier, not a native MIC or
    // recorder. This isolates real Engine segmentation of the playback SRC.
    let (sink, mut consumer) = audio_bridge::<S>(2, 64, BUDGET).unwrap();
    let mut bindings = NodeBindings::<S>::new();
    bindings.bind_source(input, source).unwrap();
    bindings.bind_sink(output, sink).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 96_000.0,
                max_block_frames: 32,
                max_events_per_block: 32,
            },
            audio_byte_budget: BUDGET,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 64,
            control_horizon_frames: 4096,
        },
    )
    .unwrap();
    assert_eq!(compiled.stats.mix_operation_count, 1);
    let gain = compiled.bindings.edge(edge).unwrap().gain;
    let content = pcm(384);
    producer.write_interleaved_exact(&content).unwrap();
    for block in 0..16_u64 {
        let start = block * 32;
        // Unchanged unity gain still requires real parameter-event segments;
        // no test wrapper directly calls or changes the production boundary.
        for offset in 1..32 {
            let at = start + offset;
            let reply = compiled.control.submit(
                ParameterRequest {
                    request_id: at,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    target: gain,
                    at: ApplyAt::Frame(at),
                    value: ParamValue::Float(1.0),
                    ramp_frames: 0,
                },
                start,
            );
            assert_eq!(reply.code, ReplyCode::Accepted);
        }
        let report =
            crate::test_allocation::assert_allocates_nothing(|| compiled.engine.render(32))
                .unwrap();
        assert_eq!(report.segments, 32);
        assert_eq!(report.applied_events, 31);
        let mut samples = [S::ZERO; 64];
        assert_eq!(
            consumer
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            32
        );
        for (index, sample) in samples.into_iter().enumerate() {
            let expected = if block == 0 {
                0.0
            } else if index % 2 == 0 {
                0.25
            } else {
                -0.5
            };
            assert!(
                (sample.to_f64() - expected).abs() < 1e-6,
                "block={block} sample={index} value={}",
                sample.to_f64()
            );
        }
        for at in start + 1..start + 32 {
            let reply = compiled.control.poll_applied().unwrap();
            assert_eq!(reply.effective_frame, at);
            assert_eq!(reply.code, ReplyCode::Applied);
        }
        assert!(compiled.control.poll_applied().is_none());
        let consumed = producer.available_frames() - (1024 - 384);
        producer
            .write_interleaved_exact(&content[..consumed * 2])
            .unwrap();
    }
    let sample = source_probe.snapshot();
    let events = sample.playback_events.unwrap();
    assert_eq!(events.initial_prime_muted_frames, 32);
    assert_eq!(events.recovery_prime_muted_frames, 0);
    assert_eq!(events.short_read_callbacks, 0);
    assert_eq!(sample.xruns, 0);
    assert_eq!(source_probe.src_checkpoint().unwrap().sequence, 512);
}

#[test]
fn playback_compiled_unity_bus_control_segments_do_not_create_false_starvation() {
    compiled_segments::<f32>();
    compiled_segments::<f64>();
}
