use super::*;
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::audio_bridge,
    compiler::{CompileConfig, NodeBindings, compile},
    host::BackendError,
    runtime::EngineConfig,
};

thread_local! {
    static BEFORE_RETURN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

pub(super) fn before_transfer_return() {
    if let Some(hook) = BEFORE_RETURN.with(|hook| hook.borrow_mut().take()) {
        hook();
    }
}

fn reset_at_transfer_return(epoch: Arc<AtomicU64>) {
    BEFORE_RETURN.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            epoch.fetch_add(1, Ordering::AcqRel);
        }))
    });
}

fn render_through_boundaries(
    channels: usize,
    input: &[f32],
    frames: usize,
    sample_rate: u32,
) -> Vec<f32> {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, channels).unwrap();
    let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();

    let (input_writer, input_reader) = audio_bridge(2, frames, 4096).unwrap();
    let (output_writer, mut output_reader) = audio_bridge(2, frames, 4096).unwrap();
    let mut input_writer = input_writer;
    input_writer.write_interleaved(input).unwrap();

    let mut bindings = NodeBindings::<f32>::new();
    bindings
        .bind_source(
            source,
            VirtualPlaybackSource::new(input_reader, channels, frames, sample_rate, 4096).unwrap(),
        )
        .unwrap();
    bindings
        .bind_sink(
            sink,
            VirtualMicrophoneSink::new(output_writer, channels, frames, sample_rate, 4096).unwrap(),
        )
        .unwrap();
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: f64::from(sample_rate),
                max_block_frames: frames,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: u64::from(sample_rate),
        },
    )
    .unwrap();
    let mut engine = compiled.engine;
    engine.render(frames).unwrap();

    let mut output = vec![f32::NAN; frames * 2];
    output_reader.read_interleaved(&mut output).unwrap();
    output
}

#[test]
fn mono_carrier_uses_slot_zero_and_writes_exact_zero_to_slot_one() {
    let output = render_through_boundaries(1, &[1.0, 99.0, 2.0, 88.0], 2, 48_000);
    assert_eq!(output, [1.0, 0.0, 2.0, 0.0]);
}

#[test]
fn stereo_carrier_preserves_each_slot_without_summing() {
    let output = render_through_boundaries(2, &[1.0, 10.0, 2.0, 20.0], 2, 48_000);
    assert_eq!(output, [1.0, 10.0, 2.0, 20.0]);
}

#[test]
fn virtual_boundaries_resample_between_engine_rates_and_native_48khz() {
    for rate in [44_100_u32, 48_000, 88_200, 96_000] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
        graph
            .connect(
                graph.get_node(source).unwrap().outputs()[0].id(),
                graph.get_node(sink).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
        let (input_writer, input_reader) = audio_bridge(2, 4096, 1 << 20).unwrap();
        let mut input_writer = input_writer;
        input_writer
            .write_interleaved(&[0.25, 0.0].repeat(4096))
            .unwrap();
        let (output_writer, mut output_reader) = audio_bridge(2, 4096, 1 << 20).unwrap();
        let mut bindings = NodeBindings::<f32>::new();
        bindings
            .bind_source(
                source,
                VirtualPlaybackSource::new(input_reader, 1, 256, rate, 1 << 20).unwrap(),
            )
            .unwrap();
        bindings
            .bind_sink(
                sink,
                VirtualMicrophoneSink::new(output_writer, 1, 256, rate, 1 << 20).unwrap(),
            )
            .unwrap();
        let compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: f64::from(rate),
                    max_block_frames: 256,
                    max_events_per_block: 8,
                },
                audio_byte_budget: 1 << 20,
                plan_revision: 1,
                timeline_epoch: 1,
                control_capacity: 8,
                control_horizon_frames: u64::from(rate),
            },
        )
        .unwrap();
        let mut engine = compiled.engine;
        for _ in 0..8 {
            engine.render(256).unwrap();
        }

        let mut samples = vec![f32::NAN; 4096 * 2];
        let report = output_reader.read_interleaved(&mut samples).unwrap();
        let expected = (2048.0 - 32.0) * 48_000.0 / f64::from(rate);
        assert!(
            (report.transferred_frames as f64 - expected).abs() < 48.0,
            "rate={rate} transferred={} expected~{expected}",
            report.transferred_frames
        );
        let valid_frames = ((report.transferred_frames - 32) / 256) * 256;
        let audio_samples: Vec<f32> = (128..valid_frames)
            .step_by(256)
            .map(|frame| samples[frame * 2])
            .collect();
        assert!(
            audio_samples
                .iter()
                .all(|sample| (*sample - 0.25).abs() < 0.01),
            "rate={rate} audio={audio_samples:?}"
        );
    }
}

#[test]
fn undersupplied_playback_is_silence_filled_and_remains_bounded() {
    let output = render_through_boundaries(1, &[3.0, 99.0], 2, 48_000);
    assert_eq!(output, [3.0, 0.0, 0.0, 0.0]);
}

#[test]
fn carrier_layout_and_staging_limits_are_rejected_before_allocation() {
    let (writer, _reader) = audio_bridge(1, 8, 4096).unwrap();
    assert!(matches!(
        VirtualMicrophoneSink::new(writer, 1, 8, 48_000, 4096),
        Err(BackendError::Unsupported(_))
    ));

    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    assert!(matches!(
        VirtualMicrophoneSink::new(writer, 1, 8, 48_000, 127),
        Err(BackendError::Unsupported(_))
    ));

    for rate in [0, 7_999, 384_001] {
        let (writer, reader) = audio_bridge(2, 512, 4096).unwrap();
        assert!(VirtualPlaybackSource::new(reader, 1, 128, rate, 1 << 20).is_err());
        assert!(VirtualMicrophoneSink::new(writer, 1, 128, rate, 1 << 20).is_err());
    }
    let (writer, reader) = audio_bridge(2, 512, 4096).unwrap();
    assert!(VirtualPlaybackSource::new(reader, 1, 128, 44_100, 4096).is_err());
    assert!(VirtualMicrophoneSink::new(writer, 1, 128, 44_100, 4096).is_err());
}

fn graph_with_bindings(
    rate: u32,
    source: impl RtAudioSource<f32> + 'static,
    sink: impl RtAudioSink<f32> + 'static,
) -> moiren_engine::runtime::Engine<f32> {
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2).unwrap();
    let output = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(input).unwrap().outputs()[0].id(),
            graph.get_node(output).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source).unwrap();
    bindings.bind_sink(output, sink).unwrap();
    compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: f64::from(rate),
                max_block_frames: 128,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1 << 20,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: u64::from(rate),
        },
    )
    .unwrap()
    .engine
}

#[test]
fn empty_playback_advances_filter_history_before_pcm_returns() {
    let (mut writer, reader) = audio_bridge(2, 512, 4096).unwrap();
    let source = VirtualPlaybackSource::new(reader, 2, 128, 44_100, 1 << 20).unwrap();
    let (sink, mut output) = audio_bridge(2, 128, 4096).unwrap();
    let mut engine = graph_with_bindings(44_100, source, sink);
    writer.write_interleaved(&[0.75; 1024]).unwrap();
    let mut scratch = [f32::NAN; 256];
    for _ in 0..24 {
        engine.render(128).unwrap();
        output.read_interleaved(&mut scratch).unwrap();
    }
    assert_eq!(scratch, [0.0; 256]);
    writer.write_interleaved(&[0.0; 1024]).unwrap();
    engine.render(128).unwrap();
    output.read_interleaved(&mut scratch).unwrap();
    assert_eq!(
        scratch, [0.0; 256],
        "idle PCM must not reappear after a gap"
    );
}

#[test]
fn microphone_backpressure_discards_fir_history_before_the_consumer_returns() {
    let (mut input, source) = audio_bridge(2, 512, 4096).unwrap();
    let (writer, mut output) = audio_bridge(2, 256, 4096).unwrap();
    let sink = VirtualMicrophoneSink::new(writer, 2, 128, 44_100, 1 << 20).unwrap();
    let mut engine = graph_with_bindings(44_100, source, sink);
    input.write_interleaved(&[0.75; 264 * 2]).unwrap();
    engine.render(128).unwrap();
    engine.render(128).unwrap();
    engine.render(8).unwrap();
    assert_eq!(output.discard(), 256);
    engine.render(64).unwrap();
    let mut scratch = [f32::NAN; 256];
    let report = output.read_interleaved(&mut scratch).unwrap();
    assert!(report.transferred_frames > 0);
    assert_eq!(scratch, [0.0; 256], "backpressure must not retain old PCM");
}

#[test]
fn microphone_stream_reset_discards_preceding_filter_lookahead() {
    let epoch = Arc::new(AtomicU64::new(0));
    let (mut input, source) = audio_bridge(2, 128, 4096).unwrap();
    let (writer, mut output) = audio_bridge(2, 256, 4096).unwrap();
    let sink = VirtualMicrophoneSink::new(writer, 2, 128, 44_100, 1 << 20)
        .unwrap()
        .with_reset_epoch(epoch.clone());
    let mut engine = graph_with_bindings(44_100, source, sink);
    input.write_interleaved(&[0.75; 8 * 2]).unwrap();
    engine.render(8).unwrap();
    output.discard();
    epoch.fetch_add(1, Ordering::AcqRel);
    engine.render(128).unwrap();
    let mut scratch = [f32::NAN; 256 * 2];
    assert!(
        output
            .read_interleaved(&mut scratch)
            .unwrap()
            .transferred_frames
            > 0
    );
    assert!(scratch.iter().all(|&sample| sample == 0.0));
}

#[test]
fn clock_drift_is_opt_in_and_admits_fir_storage_off_realtime() {
    let (writer, reader) = audio_bridge(2, 1024, 8192).unwrap();
    let source = VirtualPlaybackSource::new(reader, 1, 128, 48_000, 8192 + 1024).unwrap();
    assert!(
        source.resampler.is_none(),
        "same-rate new remains an exact copy"
    );
    assert!(
        source.with_clock_drift().is_err(),
        "FIR needs an admitted budget"
    );
    let sink = VirtualMicrophoneSink::new(writer, 1, 128, 48_000, 8192 + 1024).unwrap();
    assert!(sink.with_clock_drift().is_err());

    let (writer, reader) = audio_bridge(2, 1024, 8192).unwrap();
    let source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    let sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    assert!(source.resampler.is_some());
    assert!(sink.resampler.is_some());

    let (writer, reader) = audio_bridge(2, 128, 4096).unwrap();
    assert!(
        VirtualPlaybackSource::new(reader, 1, 128, 48_000, 1 << 20)
            .unwrap()
            .with_clock_drift()
            .is_err(),
        "no unadmitted drift headroom"
    );
    assert!(
        VirtualMicrophoneSink::new(writer, 1, 128, 48_000, 1 << 20)
            .unwrap()
            .with_clock_drift()
            .is_err()
    );
}

#[test]
fn drift_reuses_the_existing_ring_and_idle_callbacks_remain_bounded() {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    let (writer, reader) = audio_bridge(2, 512, 1 << 20).unwrap();
    let mut source = VirtualPlaybackSource::new(reader, 1, 128, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    let mut sink = VirtualMicrophoneSink::new(writer, 1, 128, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    assert_eq!(source.carrier.capacity_frames(), 512);
    assert_eq!(sink.carrier.capacity_frames(), 512);
    let mut arena = BufferArena::<f64>::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 128,
        }],
        1 << 20,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    let ctx = ProcessContext {
        timeline_epoch: 1,
        timeline_start: 0,
        frames: 128,
        processing_sr: 48_000.0,
    };
    arena
        .with_io(&write, 0..128, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                unreachable!()
            };
            let report = source.read(&ctx, outputs.get_mut(0).unwrap());
            assert_eq!(report.transferred_frames, 0);
            assert!(
                outputs
                    .get_mut(0)
                    .unwrap()
                    .channel(0)
                    .iter()
                    .all(|&s| s == 0.0)
            );
        })
        .unwrap();
    arena
        .with_io(&read, 0..128, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                unreachable!()
            };
            assert_eq!(
                sink.write(&ctx, inputs.get(0).unwrap()).transferred_frames,
                128
            );
        })
        .unwrap();
    assert!(source.carrier.available_frames() <= 128);
    assert!(sink.carrier.available_frames() > 512 - 128);
    assert!(!source.drift.as_ref().unwrap().primed);
    assert_eq!(source.resampler.as_ref().unwrap().queued_frames(), 0);
    assert!(sink.resampler.as_ref().unwrap().queued_frames() <= SRC_HEADROOM_FRAMES);
}

fn drift_trace<S: ProcessingSample>(rate: u32, channels: usize, ppm: i64) {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    const BLOCK: usize = 128;
    const CAPACITY: usize = 256;
    let (mut producer, reader) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    let (writer, mut consumer) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    let mut source = VirtualPlaybackSource::new(reader, channels, BLOCK, rate, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    let mut sink = VirtualMicrophoneSink::new(writer, channels, BLOCK, rate, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap();
    let transfer = source.drift.as_ref().unwrap().transfer;
    let target = source.drift.as_ref().unwrap().servo.target();
    let pcm: Vec<_> = (0..CAPACITY).flat_map(|_| [0.25, -0.5]).collect();
    // Seed only this deterministic simulation. Real playback primes content
    // from arriving native packets; no owner or gate waits for this depth.
    producer
        .write_interleaved(&pcm[..(target + transfer) * 2])
        .unwrap();
    sink.carrier
        .write_interleaved(&pcm[..(CAPACITY - target - transfer) * 2])
        .unwrap();
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels,
            capacity_frames: BLOCK,
        }],
        1 << 20,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    let mut scratch = [0.0; CAPACITY * 2];
    let denominator = u128::from(rate) * 1_000_000;
    let mut native_time = 0_u128;
    let mut previous_native = 0;
    let mut low = usize::MAX;
    let mut high = 0;
    // ~13..29 seconds, long enough for an uncorrected 200 ppm clock to
    // exhaust this ring's reserved headroom. No wall-clock sleeping.
    for block in 0..10_000 {
        let context = ProcessContext {
            timeline_epoch: 1,
            timeline_start: (block * BLOCK) as u64,
            frames: BLOCK,
            processing_sr: f64::from(rate),
        };
        let mut report = BoundaryReport::default();
        arena
            .with_io(&write, 0..BLOCK, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                let output = outputs.get_mut(0).unwrap();
                report = source.read(&context, output);
            })
            .unwrap();
        assert_eq!(
            report.transferred_frames, BLOCK,
            "playback rate={rate} channels={channels} ppm={ppm} block={block}"
        );
        assert_eq!(report.xruns, 0);
        arena
            .with_io(&read, 0..BLOCK, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                let input = inputs.get(0).unwrap();
                if block > 16 {
                    assert!(
                        input
                            .channel(0)
                            .iter()
                            .all(|s| (s.to_f64() - 0.25).abs() < 1e-6)
                    );
                    if channels == 2 {
                        assert!(
                            input
                                .channel(1)
                                .iter()
                                .all(|s| (s.to_f64() + 0.5).abs() < 1e-6)
                        );
                    }
                }
                report = sink.write(&context, input);
            })
            .unwrap();
        assert_eq!(
            report.transferred_frames, BLOCK,
            "microphone rate={rate} ppm={ppm} block={block}"
        );
        assert_eq!(report.xruns, 0);
        native_time += BLOCK as u128 * 48_000 * (1_000_000 + ppm) as u128;
        let total_native = native_time / denominator;
        let native_frames = (total_native - previous_native) as usize;
        previous_native = total_native;
        let report = consumer
            .read_interleaved(&mut scratch[..native_frames * 2])
            .unwrap();
        assert_eq!(
            report.transferred_frames, native_frames,
            "native starvation rate={rate} ppm={ppm} block={block}"
        );
        if block > 32 {
            for pair in scratch[..native_frames * 2].as_chunks::<2>().0 {
                assert!((pair[0] - 0.25).abs() < 1e-5);
                assert!((pair[1] - if channels == 1 { 0.0 } else { -0.5 }).abs() < 1e-5);
            }
        }
        assert_eq!(
            producer
                .write_interleaved(&pcm[..native_frames * 2])
                .unwrap()
                .transferred_frames,
            native_frames
        );
        let playback_fill = source.carrier.available_frames();
        let microphone_fill = consumer.available_frames();
        if block > 5000 {
            low = low.min(playback_fill).min(microphone_fill);
            high = high.max(playback_fill).max(microphone_fill);
        }
    }
    assert!(
        low > 16 && high < CAPACITY - 16,
        "rate={rate} ppm={ppm} fill={low}..{high}"
    );
    let source_correction = source
        .drift
        .as_mut()
        .unwrap()
        .correction(source.carrier.available_frames());
    let sink_correction = -sink
        .drift
        .as_mut()
        .unwrap()
        .correction(sink.carrier.available_frames());
    assert!(source_correction.signum() == (ppm as f64).signum());
    assert!(sink_correction.signum() == -(ppm as f64).signum());
    assert!(source_correction.abs() <= 2000.0 && sink_correction.abs() <= 2000.0);
    let nominal_frames = 10_000_u128 * BLOCK as u128 * 48_000 / u128::from(rate);
    assert!(
        previous_native.abs_diff(nominal_frames) > target as u128,
        "the uncorrected clock must exhaust the reserved headroom in this trace"
    );
}

#[test]
fn clock_drift_tracks_both_directions_at_all_rates_channels_and_precisions() {
    for rate in [44_100, 48_000, 88_200, 96_000] {
        for channels in [1, 2] {
            for ppm in [-200, 200] {
                drift_trace::<f32>(rate, channels, ppm);
                drift_trace::<f64>(rate, channels, ppm);
            }
        }
    }
}

#[test]
fn playback_waits_for_late_producer_before_discarding_and_adopting_epoch() {
    let requested = Arc::new(AtomicU64::new(0));
    let producer_ack = Arc::new(AtomicU64::new(0));
    let consumer_ack = Arc::new(AtomicU64::new(0));
    let (mut producer, reader) = audio_bridge(2, 512, 1 << 20).unwrap();
    let mut source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, 1 << 20)
        .unwrap()
        .with_reset_fence(
            requested.clone(),
            producer_ack.clone(),
            consumer_ack.clone(),
        );
    producer.write_interleaved(&[0.75; 256]).unwrap();
    requested.store(1, Ordering::Release);
    assert!(!source.synchronize_epoch());
    assert_eq!(
        source.carrier.available_frames(),
        128,
        "no discard before producer stops"
    );
    producer.write_interleaved(&[0.5; 256]).unwrap();
    producer_ack.store(1, Ordering::Release);
    assert!(source.synchronize_epoch());
    assert_eq!(
        source.carrier.available_frames(),
        0,
        "late old publication is included"
    );
    assert_eq!(consumer_ack.load(Ordering::Acquire), 1);
    assert_eq!(source.observed_reset, 1);
    producer.write_interleaved(&[0.25; 256]).unwrap();
    requested.store(2, Ordering::Release);
    producer_ack.store(2, Ordering::Release);
    requested.store(3, Ordering::Release);
    assert!(
        !source.synchronize_epoch(),
        "old producer ack cannot clear a newer request"
    );
    assert_eq!(source.carrier.available_frames(), 128);
}

#[test]
fn reset_racing_graph_read_silences_the_entire_output_block() {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    for rate in [44_100, 48_000] {
        let requested = Arc::new(AtomicU64::new(0));
        let producer_ack = Arc::new(AtomicU64::new(0));
        let consumer_ack = Arc::new(AtomicU64::new(0));
        let (mut producer, reader) = audio_bridge(2, 512, 1 << 20).unwrap();
        producer.write_interleaved(&[0.75; 1024]).unwrap();
        let mut source = VirtualPlaybackSource::new(reader, 1, 128, rate, 1 << 20)
            .unwrap()
            .with_reset_epoch(requested.clone())
            .with_epoch_handshake(producer_ack, consumer_ack);
        let mut arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 128,
            }],
            1 << 20,
        )
        .unwrap();
        let io = arena
            .prepare_io(&[PortAccess::Write {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        reset_at_transfer_return(requested);
        arena
            .with_io(&io, 0..128, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                let report = source.read(
                    &ProcessContext {
                        timeline_epoch: 1,
                        timeline_start: 0,
                        frames: 128,
                        processing_sr: f64::from(rate),
                    },
                    outputs.get_mut(0).unwrap(),
                );
                assert_eq!(report.transferred_frames, 0);
                assert!(
                    outputs
                        .get_mut(0)
                        .unwrap()
                        .channel(0)
                        .iter()
                        .all(|&s| s == 0.0)
                );
            })
            .unwrap();
        assert!(!source.synchronize_epoch(), "producer has not quiesced yet");
    }
}

#[test]
fn microphone_late_write_is_fenced_until_the_consumer_discards_it() {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    for rate in [44_100, 48_000] {
        let requested = Arc::new(AtomicU64::new(0));
        let producer_ack = Arc::new(AtomicU64::new(0));
        let consumer_ack = Arc::new(AtomicU64::new(0));
        let (writer, mut consumer) = audio_bridge(2, 512, 1 << 20).unwrap();
        let mut sink = VirtualMicrophoneSink::new(writer, 1, 128, rate, 1 << 20)
            .unwrap()
            .with_reset_fence(
                requested.clone(),
                producer_ack.clone(),
                consumer_ack.clone(),
            );
        let mut arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 128,
            }],
            1 << 20,
        )
        .unwrap();
        let io = arena
            .prepare_io(&[PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        reset_at_transfer_return(requested);
        arena
            .with_io(&io, 0..128, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                let report = sink.write(
                    &ProcessContext {
                        timeline_epoch: 1,
                        timeline_start: 0,
                        frames: 128,
                        processing_sr: f64::from(rate),
                    },
                    inputs.get(0).unwrap(),
                );
                assert_eq!(report.transferred_frames, 0);
            })
            .unwrap();
        assert_eq!(producer_ack.load(Ordering::Acquire), 1);
        assert!(consumer.available_frames() > 0, "the old write raced reset");
        assert!(
            !sink.synchronize_epoch(),
            "no fresh publication before consumer discard"
        );
        consumer.discard();
        consumer_ack.store(1, Ordering::Release);
        assert!(sink.synchronize_epoch());
        assert_eq!(sink.observed_reset, 1);
        assert_eq!(consumer.available_frames(), 0);
    }
}

#[test]
fn microphone_backpressure_drops_new_without_requesting_a_reset_without_src() {
    microphone_backpressure_drops_new_without_requesting_a_reset(false, 0);
}

#[test]
fn microphone_backpressure_drops_new_without_requesting_a_reset_with_clock_drift() {
    microphone_backpressure_drops_new_without_requesting_a_reset(true, 0);
}

#[test]
fn microphone_backpressure_after_partial_write_drops_new_without_requesting_a_reset_without_src() {
    microphone_backpressure_drops_new_without_requesting_a_reset(false, 32);
}

#[test]
fn microphone_backpressure_after_partial_write_drops_new_without_requesting_a_reset_with_clock_drift()
 {
    microphone_backpressure_drops_new_without_requesting_a_reset(true, 32);
}

#[test]
fn microphone_backpressure_without_a_complete_fence_keeps_drop_new_behavior() {
    for drift in [false, true] {
        for reset_only in [false, true] {
            let requested = Arc::new(AtomicU64::new(0));
            let (mut writer, mut consumer) = audio_bridge(2, 4800, 1 << 20).unwrap();
            writer.write_interleaved_exact(&[0.75; 9600]).unwrap();
            let mut sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, 1 << 20).unwrap();
            if drift {
                sink = sink.with_clock_drift().unwrap();
            }
            if reset_only {
                sink = sink.with_reset_epoch(requested.clone());
            }
            let (mut input, source) = audio_bridge(2, 128, 4096).unwrap();
            input.write_interleaved_exact(&[-0.25; 256]).unwrap();
            let mut engine = graph_with_bindings(48_000, source, sink);
            engine.render(128).unwrap();
            assert_eq!(requested.load(Ordering::Acquire), 0);
            assert_eq!(consumer.available_frames(), 4800);
            let mut scratch = [f32::NAN; 256];
            consumer.read_interleaved(&mut scratch).unwrap();
            assert_eq!(scratch, [0.75; 256]);
        }
    }
}

/// Full or nearly-full carrier overflow is bounded loss: the newest graph
/// frames are dropped, the oldest queued frames stay in order, and the
/// worker drains the carrier at its own cadence. No epoch is requested -
/// a fenced discard here used to convert bounded jitter into ~50 ms of
/// guaranteed silence plus a full worker re-handshake (the graded-reset
/// redesign: discard is reserved for real epoch events).
fn microphone_backpressure_drops_new_without_requesting_a_reset(drift: bool, free_frames: usize) {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    const CAPACITY: usize = 4800;
    const BLOCK: usize = 128;
    let requested = Arc::new(AtomicU64::new(0));
    let producer_ack = Arc::new(AtomicU64::new(0));
    let consumer_ack = Arc::new(AtomicU64::new(0));
    let (mut writer, mut consumer) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    assert!(
        writer
            .write_interleaved_exact(&vec![0.75; (CAPACITY - free_frames) * 2])
            .unwrap()
    );
    let mut sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, 48_000, 1 << 20)
        .unwrap()
        .with_reset_fence(
            requested.clone(),
            producer_ack.clone(),
            consumer_ack.clone(),
        );
    if drift {
        sink = sink.with_clock_drift().unwrap();
    }
    let mut arena = BufferArena::<f64>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: BLOCK,
        }],
        1 << 20,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    arena
        .with_io(&write, 0..BLOCK, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                unreachable!()
            };
            let mut output = outputs.get_mut(0).unwrap();
            for channel in 0..2 {
                output.channel_mut(channel).fill(-0.25);
            }
        })
        .unwrap();
    let context = ProcessContext {
        timeline_epoch: 1,
        timeline_start: 0,
        frames: BLOCK,
        processing_sr: 48_000.0,
    };
    let mut publish = |sink: &mut VirtualMicrophoneSink| {
        arena
            .with_io(&read, 0..BLOCK, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                sink.write(&context, inputs.get(0).unwrap())
            })
            .unwrap()
    };
    let report = publish(&mut sink);
    assert_eq!(consumer.available_frames(), CAPACITY, "old prefix kept");
    assert_eq!(
        requested.load(Ordering::Acquire),
        0,
        "overflow is bounded loss, not a timeline break"
    );
    assert_eq!(producer_ack.load(Ordering::Acquire), 0);
    assert_eq!(consumer_ack.load(Ordering::Acquire), 0);
    // Only the free prefix publishes; the remainder is dropped, not retained.
    // (The drift variant reports engine-frame equivalents of the published
    // prefix; its exact value depends on FIR state, so only the raw path is
    // pinned to the free-frame count.)
    if drift {
        assert!(report.transferred_frames < 128);
    } else {
        assert_eq!(report.transferred_frames, free_frames);
    }
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 1);
    assert_eq!(sink.carrier.capacity_frames(), CAPACITY);

    // Publication resumes as soon as the worker drains space: no fence to
    // complete, no discard, no re-handshake.
    let mut scratch = [f32::NAN; BLOCK * 2];
    assert!(
        consumer
            .read_interleaved(&mut scratch)
            .unwrap()
            .transferred_frames
            > 0
    );
    assert_eq!(scratch[..2], [0.75, 0.75], "oldest frames stay in order");
    for _ in 0..3 {
        assert!(publish(&mut sink).transferred_frames > 0);
        assert!(
            consumer
                .read_interleaved(&mut scratch)
                .unwrap()
                .transferred_frames
                > 0
        );
        assert_eq!(requested.load(Ordering::Acquire), 0);
        assert_eq!(producer_ack.load(Ordering::Acquire), 0);
    }
    assert_eq!(sink.observed_reset, 0);
}

#[test]
fn microphone_backpressure_preserves_a_reset_racing_the_partial_publication() {
    for drift in [false, true] {
        let requested = Arc::new(AtomicU64::new(0));
        let producer_ack = Arc::new(AtomicU64::new(0));
        let consumer_ack = Arc::new(AtomicU64::new(0));
        let (mut writer, mut consumer) = audio_bridge(2, 4800, 1 << 20).unwrap();
        writer.write_interleaved_exact(&[0.75; 9600]).unwrap();
        let mut sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, 1 << 20)
            .unwrap()
            .with_reset_fence(
                requested.clone(),
                producer_ack.clone(),
                consumer_ack.clone(),
            );
        if drift {
            sink = sink.with_clock_drift().unwrap();
        }
        let (mut input, source) = audio_bridge(2, 128, 4096).unwrap();
        input.write_interleaved_exact(&[-0.25; 256]).unwrap();
        let mut engine = graph_with_bindings(48_000, source, sink);
        let reset = requested.clone();
        let ack = producer_ack.clone();
        BEFORE_RETURN.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                assert_eq!(
                    ack.load(Ordering::Acquire),
                    0,
                    "old write is not fenced yet"
                );
                reset.store(2, Ordering::Release);
            }));
        });
        engine.render(128).unwrap();
        // The externally requested reset wins over the completed publication:
        // the producer quiesces on the same write and nothing further
        // publishes until the consumer discards and acknowledges.
        assert_eq!(requested.load(Ordering::Acquire), 2);
        assert_eq!(producer_ack.load(Ordering::Acquire), 2);
        assert_eq!(consumer_ack.load(Ordering::Acquire), 0);
        engine.render(128).unwrap();
        assert_eq!(
            consumer.available_frames(),
            4800,
            "publication stays fenced"
        );
        assert_eq!(consumer.discard(), 4800);
        consumer_ack.store(2, Ordering::Release);
        engine.render(128).unwrap();
        let mut scratch = [f32::NAN; 256];
        assert!(
            consumer
                .read_interleaved(&mut scratch)
                .unwrap()
                .transferred_frames
                > 0,
            "fresh content flows after the fence completes"
        );
    }
}

#[test]
fn playback_starvation_keeps_the_settled_servo_state() {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    const CAPACITY: usize = 4800;
    const BLOCK: usize = 128;
    let (mut producer, reader) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    let probe = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, BLOCK, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap()
        .with_probe(probe.clone());
    let transfer = source.drift.as_ref().unwrap().transfer;
    let target = source.drift.as_ref().unwrap().servo.target();
    let pcm: Vec<f32> = (0..CAPACITY).flat_map(|_| [0.25, -0.5]).collect();
    // Seed only this deterministic simulation; real playback primes from
    // arriving native packets.
    producer
        .write_interleaved(&pcm[..(target + transfer) * 2])
        .unwrap();
    let mut arena = BufferArena::<f32>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: BLOCK,
        }],
        1 << 20,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    let context = |block: usize| ProcessContext {
        timeline_epoch: 1,
        timeline_start: (block * BLOCK) as u64,
        frames: BLOCK,
        processing_sr: 48_000.0,
    };
    let mut read_block = |source: &mut VirtualPlaybackSource, block: usize| {
        let mut report = BoundaryReport::default();
        arena
            .with_io(&write, 0..BLOCK, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                let output = outputs.get_mut(0).unwrap();
                report = source.read(&context(block), output);
            })
            .unwrap();
        report
    };
    // Settle the servo clearly positive by holding the level near the ring
    // top: relative error ~+1, so the correction converges toward the clamp.
    producer
        .write_interleaved(&pcm[(target + transfer) * 2..CAPACITY * 2])
        .unwrap();
    for block in 0..300 {
        let report = read_block(&mut source, block);
        assert_eq!(report.transferred_frames, BLOCK);
        assert_eq!(report.xruns, 0);
        producer.write_interleaved(&pcm[..BLOCK * 2]).unwrap();
    }
    let settled = source.drift.as_ref().unwrap().servo.correction();
    assert!(
        settled > 1000.0,
        "servo must settle well inside its authority, got {settled} ppm"
    );
    assert!(source.drift.as_ref().unwrap().primed);
    let sample = probe.snapshot();
    assert!(
        sample.level_frames > 2_000,
        "probe sees the queued occupancy: {sample:?}"
    );
    assert!(
        sample.applied_milli_ppm > 1_000_000,
        "probe sees the settled correction: {sample:?}"
    );
    // The initial prime's recovery edge is the only soft loss so far.
    assert_eq!(sample.soft_events, SRC_HEADROOM_FRAMES as u64);
    assert_eq!(sample.xruns, 0);
    // Drain the carrier and starve one block: an honest counted xrun... The
    // drain itself legitimately walks the correction partway back down.
    while source.carrier.available_frames() > 0 {
        read_block(&mut source, 300);
    }
    let pre_starve = source.drift.as_ref().unwrap().servo.correction();
    let starved = read_block(&mut source, 301);
    assert!(starved.xruns >= 1, "starvation is counted");
    assert!(starved.discontinuity);
    let sample = probe.snapshot();
    assert!(
        sample.soft_events >= 1,
        "probe counts the soft starvation loss: {sample:?}"
    );
    assert!(sample.xruns >= 1);
    // ...but the servo's settled correction survives while the prime gate
    // re-arms on a full starvation: consumption pauses until the level
    // rebuilds (bounded, no discard) instead of riding marginal supply for
    // ~20 s of servo-only recovery.
    let after = source.drift.as_ref().unwrap().servo.correction();
    assert!(
        after > 500.0,
        "starvation must not discard the settled correction: {after} ppm"
    );
    assert!(
        (after - pre_starve).abs() < 100.0,
        "the starved block may only move the servo one smoothing step: \
         {pre_starve} -> {after} ppm"
    );
    assert!(
        !source.drift.as_ref().unwrap().primed,
        "a fully starved block re-arms the prime gate"
    );
    // Below the prime threshold consumption stays paused; reaching
    // transfer+target resumes cleanly with the servo state intact.
    producer.write_interleaved(&pcm[..BLOCK * 4 * 2]).unwrap();
    let held = read_block(&mut source, 302);
    assert_eq!(
        held.transferred_frames, 0,
        "unprimed blocks render held silence, not carrier content"
    );
    assert_eq!(
        source.carrier.available_frames(),
        BLOCK * 4,
        "unprimed blocks hold the carrier, not read it"
    );
    let transfer = source.drift.as_ref().unwrap().transfer;
    producer
        .write_interleaved(&pcm[..(transfer + target - BLOCK * 4) * 2])
        .unwrap();
    let recovered = read_block(&mut source, 303);
    assert_eq!(recovered.transferred_frames, BLOCK);
    assert_eq!(recovered.xruns, 0);
    assert!(source.drift.as_ref().unwrap().primed);
    assert!(source.drift.as_ref().unwrap().servo.correction() > 500.0);
    // The resumption edge is exact zero for one FIR length - never a
    // mixed-amplitude frame downstream - then real content resumes.
    let mut samples = [0.0f32; BLOCK * 2];
    arena
        .with_io(&read, 0..BLOCK, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                unreachable!()
            };
            let input = inputs.get(0).unwrap();
            for frame in 0..BLOCK {
                samples[frame * 2] = input.channel(0)[frame];
                samples[frame * 2 + 1] = input.channel(1)[frame];
            }
        })
        .unwrap();
    let (mute, rest) = samples.split_at_mut(SRC_HEADROOM_FRAMES * 2);
    assert_eq!(&mute[..8], &[0.0; 8]);
    assert!((rest[0] - 0.25).abs() < 1e-6 && (rest[1] + 0.5).abs() < 1e-6);
    assert!(probe.snapshot().soft_events >= 2);
}

#[test]
fn microphone_probe_reports_occupancy_and_overflow() {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
    const CAPACITY: usize = 4800;
    const BLOCK: usize = 128;
    let (writer, consumer) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    let probe = Arc::new(BoundaryProbe::default());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, 48_000, 1 << 20)
        .unwrap()
        .with_clock_drift()
        .unwrap()
        .with_probe(probe.clone());
    let mut arena = BufferArena::<f32>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: BLOCK,
        }],
        1 << 20,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    let context = |block: usize| ProcessContext {
        timeline_epoch: 1,
        timeline_start: (block * BLOCK) as u64,
        frames: BLOCK,
        processing_sr: 48_000.0,
    };
    let mut publish = |sink: &mut VirtualMicrophoneSink, block: usize| {
        arena
            .with_io(&write, 0..BLOCK, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                for channel in 0..2 {
                    outputs.get_mut(0).unwrap().channel_mut(channel).fill(-0.25);
                }
            })
            .unwrap();
        arena
            .with_io(&read, 0..BLOCK, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                sink.write(&context(block), inputs.get(0).unwrap())
            })
            .unwrap()
    };
    publish(&mut sink, 0);
    assert!(
        probe.snapshot().level_frames > 0,
        "the probe reports queued occupancy, not free space"
    );
    // Fill to the ring top.
    for block in 1..60 {
        publish(&mut sink, block);
        if sink.carrier.available_frames() == 0 {
            break;
        }
    }
    assert_eq!(probe.snapshot().level_frames, CAPACITY as u32);
    // One more write overflows: the probe counts the soft loss while the
    // queued frames stay in place.
    publish(&mut sink, 60);
    let sample = probe.snapshot();
    assert_eq!(sample.level_frames, CAPACITY as u32);
    assert!(sample.soft_events >= 1, "{sample:?}");
    assert!(sample.xruns >= 1, "{sample:?}");
    assert_eq!(consumer.available_frames(), CAPACITY, "old prefix kept");
}
