use super::*;
use moiren_engine::{
    boundary::audio_bridge,
    buffer::{BufferArena, BufferSlotLayout, PortAccess, PreparedIo, ProcessIo},
};

const BUDGET: usize = 1 << 20;

#[test]
fn probe_preparation_metadata_is_total_content_and_builder_order_independent() {
    let empty = BoundaryProbe::default().snapshot();
    assert_eq!(empty.capacity_frames, 0);
    assert_eq!(empty.target_frames, None);
    for capacity in [1024, 8192] {
        for rate in [44_100, 48_000, 96_000] {
            for mode in 0..3 {
                for probe_first in [false, true] {
                    let (_producer, reader) = audio_bridge(2, capacity, BUDGET).unwrap();
                    let (writer, _consumer) = audio_bridge(2, capacity, BUDGET).unwrap();
                    let playback = Arc::new(BoundaryProbe::default());
                    let microphone = Arc::new(BoundaryProbe::default());
                    let mut source =
                        VirtualPlaybackSource::new(reader, 2, 128, rate, BUDGET).unwrap();
                    let mut sink =
                        VirtualMicrophoneSink::new(writer, 2, 128, rate, BUDGET).unwrap();
                    if probe_first {
                        source = source.with_probe(playback.clone());
                        sink = sink.with_probe(microphone.clone());
                    }
                    if mode == 1 {
                        source = source.with_clock_drift().unwrap();
                        sink = sink.with_clock_drift().unwrap();
                    } else if mode == 2 {
                        source = source.with_clock_drift_target(384).unwrap();
                        sink = sink.with_clock_drift_target(384).unwrap();
                    }
                    if !probe_first {
                        source = source.with_probe(playback.clone());
                        sink = sink.with_probe(microphone.clone());
                    }
                    let transfer = drift_transfer(128, rate).unwrap();
                    let legacy_reference = transfer + (capacity - transfer) / 2;
                    let expected_playback = match mode {
                        0 => None,
                        1 => Some(legacy_reference as u32),
                        _ => Some(384),
                    };
                    let expected_microphone = match mode {
                        0 => None,
                        1 => Some((capacity - legacy_reference) as u32),
                        _ => Some(384),
                    };
                    for (probe, target) in [
                        (&playback, expected_playback),
                        (&microphone, expected_microphone),
                    ] {
                        let sample = probe.snapshot();
                        assert_eq!(sample.capacity_frames, capacity as u32);
                        assert_eq!(sample.target_frames, target);
                        assert_eq!(sample.level_frames, 0, "preparation is not a live sample");
                    }
                    assert_eq!(source.carrier.capacity_frames(), capacity);
                    assert_eq!(sink.carrier.capacity_frames(), capacity);
                }
            }
        }
    }
}

#[test]
fn probe_preparation_metadata_survives_rt_shortage_overflow_and_epoch_reset() {
    let (_producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let (writer, _consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let epoch = Arc::new(AtomicU64::new(0));
    let mut source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, BUDGET)
        .unwrap()
        .with_probe(playback.clone())
        .with_clock_drift_target(384)
        .unwrap()
        .with_reset_epoch(epoch.clone());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, BUDGET)
        .unwrap()
        .with_probe(microphone.clone())
        .with_clock_drift_target(384)
        .unwrap()
        .with_reset_epoch(epoch.clone());
    let mut buffer = CallbackBuffer::<f32>::new(128, 48_000);
    for callback in 0..32 {
        if callback == 16 {
            epoch.store(1, Ordering::Release);
        }
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
        buffer.write_sink(&mut sink);
        for probe in [&playback, &microphone] {
            let sample = probe.snapshot();
            assert_eq!(sample.capacity_frames, 1024);
            assert_eq!(sample.target_frames, Some(384));
        }
    }
    assert_eq!(microphone.snapshot().level_frames, 1024);
    assert!(microphone.snapshot().xruns > 0);
}

#[test]
fn rejected_drift_preparation_does_not_publish_an_effective_target() {
    let (writer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, 8192 + 1024)
        .unwrap()
        .with_probe(playback.clone());
    let sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, 8192 + 1024)
        .unwrap()
        .with_probe(microphone.clone());
    assert!(source.with_clock_drift_target(384).is_err());
    assert!(sink.with_clock_drift_target(384).is_err());
    for probe in [&playback, &microphone] {
        assert_eq!(probe.snapshot().capacity_frames, 1024);
        assert_eq!(probe.snapshot().target_frames, None);
    }
}

struct CallbackBuffer<S: ProcessingSample> {
    arena: BufferArena<S>,
    write: PreparedIo,
    read: PreparedIo,
    context: ProcessContext,
}

impl<S: ProcessingSample> CallbackBuffer<S> {
    fn new(frames: usize, rate: u32) -> Self {
        let arena = BufferArena::new(
            &[BufferSlotLayout {
                channels: 2,
                capacity_frames: frames,
            }],
            BUDGET,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let write = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot }])
            .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot }])
            .unwrap();
        Self {
            arena,
            write,
            read,
            context: ProcessContext {
                timeline_epoch: 1,
                timeline_start: 0,
                frames,
                processing_sr: f64::from(rate),
            },
        }
    }

    fn read_source(&mut self, source: &mut VirtualPlaybackSource) -> BoundaryReport {
        self.arena
            .with_io(&self.write, 0..self.context.frames, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                source.read(&self.context, outputs.get_mut(0).unwrap())
            })
            .unwrap()
    }

    fn write_sink(&mut self, sink: &mut VirtualMicrophoneSink) -> BoundaryReport {
        self.arena
            .with_io(&self.read, 0..self.context.frames, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                sink.write(&self.context, inputs.get(0).unwrap())
            })
            .unwrap()
    }

    fn assert_pcm(&mut self, muted: usize) {
        self.assert_values(muted, [0.25, -0.5]);
    }

    fn fill(&mut self, values: [f64; 2]) {
        self.arena
            .with_io(&self.write, 0..self.context.frames, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                for (channel, value) in values.into_iter().enumerate() {
                    outputs
                        .get_mut(0)
                        .unwrap()
                        .channel_mut(channel)
                        .fill(S::from_f64(value));
                }
            })
            .unwrap();
    }

    fn assert_values(&mut self, muted: usize, values: [f64; 2]) {
        self.arena
            .with_io(&self.read, 0..self.context.frames, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                let input = inputs.get(0).unwrap();
                for (channel, expected) in values.into_iter().enumerate() {
                    for (frame, &sample) in input.channel(channel).iter().enumerate() {
                        let expected = if frame < muted { 0.0 } else { expected };
                        assert!(
                            (sample.to_f64() - expected).abs() < 1e-6,
                            "channel={channel} frame={frame} sample={} expected={expected}",
                            sample.to_f64()
                        );
                    }
                }
            })
            .unwrap();
    }
}

fn pcm(frames: usize) -> Vec<f32> {
    (0..frames).flat_map(|_| [0.25, -0.5]).collect()
}

#[test]
fn explicit_target_admits_only_content_with_both_transfer_reserves() {
    for (rate, expected_transfer) in [(44_100, 172), (48_000, 161), (96_000, 97)] {
        const CAPACITY: usize = 1024;
        assert_eq!(drift_transfer(128, rate).unwrap(), expected_transfer);
        let minimum = expected_transfer + SRC_HEADROOM_FRAMES;
        let maximum = CAPACITY - expected_transfer;
        for target in [
            0,
            SRC_HEADROOM_FRAMES,
            expected_transfer,
            minimum - 1,
            minimum,
            maximum,
            maximum + 1,
            CAPACITY,
            usize::MAX,
        ] {
            let (writer, reader) = audio_bridge(2, CAPACITY, BUDGET).unwrap();
            let source = VirtualPlaybackSource::new(reader, 2, 128, rate, BUDGET)
                .unwrap()
                .with_clock_drift_target(target);
            let sink = VirtualMicrophoneSink::new(writer, 2, 128, rate, BUDGET)
                .unwrap()
                .with_clock_drift_target(target);
            let admitted = (minimum..=maximum).contains(&target);
            assert_eq!(source.is_ok(), admitted, "playback {rate} target={target}");
            assert_eq!(sink.is_ok(), admitted, "microphone {rate} target={target}");
        }
    }
}

#[test]
fn explicit_target_keeps_fir_budget_admission_and_checked_geometry() {
    let (writer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, 8192 + 1024).unwrap();
    let sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, 8192 + 1024).unwrap();
    assert!(source.with_clock_drift_target(384).is_err());
    assert!(sink.with_clock_drift_target(384).is_err());
    assert!(drift_transfer(usize::MAX, 8_000).is_err());
}

fn capacity_independent_trace<S: ProcessingSample>(
    capacity: usize,
    rate: u32,
) -> Vec<(usize, usize)> {
    const BLOCK: usize = 128;
    const TARGET: usize = 384;
    let (mut producer, reader) = audio_bridge(2, capacity, BUDGET).unwrap();
    let (mut writer, mut consumer) = audio_bridge(2, capacity, BUDGET).unwrap();
    let content = pcm(capacity);
    writer
        .write_interleaved_exact(&content[..TARGET * 2])
        .unwrap();
    let probe = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(TARGET)
        .unwrap()
        .with_probe(probe.clone());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(TARGET)
        .unwrap();
    let mut buffer = CallbackBuffer::<S>::new(BLOCK, rate);
    producer
        .write_interleaved_exact(&content[..(TARGET - 1) * 2])
        .unwrap();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
    buffer.assert_pcm(BLOCK);
    assert_eq!(source.carrier.available_frames(), TARGET - 1);
    producer.write_interleaved_exact(&content[..2]).unwrap();
    let mut trace = Vec::new();
    let mut native_output = vec![0.0; capacity * 2];
    let mut drained = 0;
    let stable_after =
        TARGET + missing_output_frames(SRC_HEADROOM_FRAMES * 2, rate, NATIVE_SAMPLE_RATE);
    for block in 0..128 {
        let report = buffer.read_source(&mut source);
        assert_eq!(report.transferred_frames, BLOCK);
        assert_eq!(report.xruns, 0);
        buffer.assert_pcm(if block == 0 { SRC_HEADROOM_FRAMES } else { 0 });
        let consumed = TARGET - source.carrier.available_frames();
        assert_eq!(probe.snapshot().level_frames as usize, TARGET - consumed);
        assert_eq!(source.drift.as_ref().unwrap().servo.correction(), 0.0);
        producer
            .write_interleaved_exact(&content[..consumed * 2])
            .unwrap();
        let report = buffer.write_sink(&mut sink);
        assert_eq!(report.transferred_frames, BLOCK);
        assert_eq!(report.xruns, 0);
        let published = consumer.available_frames() - TARGET;
        assert_eq!(sink.drift.as_ref().unwrap().servo.correction(), 0.0);
        consumer
            .read_interleaved(&mut native_output[..published * 2])
            .unwrap();
        for (offset, frame) in native_output[..published * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
        {
            // The seeded prefix precedes the source's exact-zero recovery
            // edge and the sink FIR's response to that edge.
            if drained + offset >= stable_after {
                assert!((frame[0] - 0.25).abs() < 1e-6);
                assert!((frame[1] + 0.5).abs() < 1e-6);
            }
        }
        drained += published;
        trace.push((consumed, published));
    }
    trace
}

#[test]
fn explicit_target_primes_and_servos_at_the_same_content_for_different_capacities() {
    for rate in [44_100, 48_000, 88_200, 96_000] {
        assert_eq!(
            capacity_independent_trace::<f32>(1024, rate),
            capacity_independent_trace::<f32>(8192, rate),
            "rate={rate}"
        );
        assert_eq!(
            capacity_independent_trace::<f64>(1024, rate),
            capacity_independent_trace::<f64>(8192, rate),
            "rate={rate}"
        );
    }
}

#[test]
fn explicit_microphone_target_uses_content_occupancy_with_the_drain_sign() {
    for capacity in [1024, 8192] {
        for occupancy in [256, 384, 512] {
            let (mut writer, _consumer) = audio_bridge(2, capacity, BUDGET).unwrap();
            writer.write_interleaved_exact(&pcm(occupancy)).unwrap();
            let mut sink = VirtualMicrophoneSink::new(writer, 2, 128, 48_000, BUDGET)
                .unwrap()
                .with_clock_drift_target(384)
                .unwrap();
            let probe = Arc::new(BoundaryProbe::default());
            sink = sink.with_probe(probe.clone());
            let mut buffer = CallbackBuffer::<f32>::new(128, 48_000);
            assert_eq!(buffer.write_sink(&mut sink).transferred_frames, 128);
            let correction = sink.drift.as_ref().unwrap().servo.correction();
            assert_eq!(
                correction.total_cmp(&0.0),
                (occupancy as f64).total_cmp(&384.0),
                "capacity={capacity} occupancy={occupancy} correction={correction}"
            );
            assert_eq!(
                probe.snapshot().applied_milli_ppm,
                (correction * 1000.0) as i32
            );
        }
    }
}

#[test]
fn explicit_target_checked_admission_covers_extreme_engine_rates() {
    for rate in [8_000, 384_000] {
        let transfer = drift_transfer(256, rate).unwrap();
        let target = transfer + SRC_HEADROOM_FRAMES;
        let (writer, reader) = audio_bridge(2, 8192, BUDGET).unwrap();
        let source = VirtualPlaybackSource::new(reader, 1, 256, rate, BUDGET)
            .unwrap()
            .with_clock_drift_target(target)
            .unwrap();
        let sink = VirtualMicrophoneSink::new(writer, 1, 256, rate, BUDGET)
            .unwrap()
            .with_clock_drift_target(target)
            .unwrap();
        assert_eq!(source.drift.unwrap().servo.target(), SRC_HEADROOM_FRAMES);
        assert_eq!(sink.drift.unwrap().servo.target(), SRC_HEADROOM_FRAMES);
    }
    // These synthetic layouts cannot be allocated as carriers; exercise the
    // same checked target admission without trying to reserve their storage.
    assert!(ClockDrift::with_target(usize::MAX, 128, usize::MAX - 64).is_err());
    assert!(ClockDrift::with_target(usize::MAX, usize::MAX, 128).is_err());
    assert!(drift_transfer(128, 0).is_err());
}

fn packet_phase_trace(
    capacity: usize,
    rate: u32,
    block: usize,
    packet: usize,
    phase: usize,
    ppm: i64,
    callbacks: usize,
) -> Vec<(usize, usize, i32, i32)> {
    let transfer = drift_transfer(block, rate).unwrap();
    let target = transfer + packet + SRC_HEADROOM_FRAMES;
    let (mut producer, reader) = audio_bridge(2, capacity, BUDGET).unwrap();
    let (mut writer, mut consumer) = audio_bridge(2, capacity, BUDGET).unwrap();
    let content = pcm(capacity);
    producer
        .write_interleaved_exact(&content[..target * 2])
        .unwrap();
    writer
        .write_interleaved_exact(&content[..target * 2])
        .unwrap();
    let playback_probe = Arc::new(BoundaryProbe::default());
    let microphone_probe = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, block, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(target)
        .unwrap()
        .with_probe(playback_probe.clone());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, block, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(target)
        .unwrap()
        .with_probe(microphone_probe.clone());
    let mut buffer = CallbackBuffer::<f32>::new(block, rate);
    let mut scratch = vec![0.0; packet * 2];
    let mut native_time = 0_u128;
    let denominator = u128::from(rate) * 1_000_000;
    let mut next_packet = phase;
    let mut drained = 0;
    let mut trace = Vec::new();
    let stable_after =
        target + missing_output_frames(SRC_HEADROOM_FRAMES * 2, rate, NATIVE_SAMPLE_RATE);
    for callback in 0..callbacks {
        while next_packet as u128 <= native_time / denominator {
            let received = producer.write_interleaved(&content[..packet * 2]).unwrap();
            assert_eq!(received.transferred_frames, packet, "native overflow");
            let sent = consumer.read_interleaved(&mut scratch).unwrap();
            assert_eq!(sent.transferred_frames, packet, "native starvation");
            for (frame, samples) in scratch.as_chunks::<2>().0.iter().enumerate() {
                if drained + frame >= stable_after {
                    assert!((samples[0] - 0.25).abs() < 1e-6);
                    assert!((samples[1] + 0.5).abs() < 1e-6);
                }
            }
            drained += packet;
            next_packet += packet;
        }
        let report = buffer.read_source(&mut source);
        assert_eq!(
            report.transferred_frames, block,
            "rate={rate} block={block} packet={packet} phase={phase} ppm={ppm} callback={callback}"
        );
        assert_eq!(report.xruns, 0);
        buffer.assert_pcm(if callback == 0 {
            SRC_HEADROOM_FRAMES
        } else {
            0
        });
        let report = buffer.write_sink(&mut sink);
        assert_eq!(
            report.transferred_frames,
            block,
            "microphone capacity={capacity} rate={rate} block={block} packet={packet} \
             phase={phase} ppm={ppm} callback={callback} target={target} \
             queued={} report={report:?}",
            consumer.available_frames()
        );
        assert_eq!(report.xruns, 0);
        let playback = playback_probe.snapshot();
        let microphone = microphone_probe.snapshot();
        let excursion = packet + transfer + SRC_HEADROOM_FRAMES;
        for level in [playback.level_frames, microphone.level_frames] {
            assert!(
                (level as usize).abs_diff(target) <= excursion,
                "rate={rate} block={block} packet={packet} phase={phase} level={level} target={target}"
            );
            assert!((level as usize) < capacity);
        }
        assert_eq!(playback.xruns + microphone.xruns, 0);
        assert_eq!(playback.soft_events, SRC_HEADROOM_FRAMES as u64);
        assert_eq!(microphone.soft_events, 0);
        trace.push((
            playback.level_frames as usize,
            microphone.level_frames as usize,
            playback.applied_milli_ppm,
            microphone.applied_milli_ppm,
        ));
        native_time += block as u128 * 48_000 * (1_000_000 + ppm) as u128;
    }
    trace
}

#[test]
fn explicit_target_packet_block_phase_matrix_is_bounded_in_both_directions() {
    let mut admitted = 0;
    let mut rejected = 0;
    let mut packet_rejected = 0;
    for rate in [44_100, 48_000, 96_000] {
        for block in [64, 128, 256] {
            for packet in [64, 128, 240, 256, 480] {
                let transfer = drift_transfer(block, rate).unwrap();
                let target = transfer + packet + SRC_HEADROOM_FRAMES;
                for phase in [0, packet / 2, packet - 1] {
                    for ppm in [-200, 0, 200] {
                        let large = packet_phase_trace(8192, rate, block, packet, phase, ppm, 128);
                        if target + transfer + packet <= 1024 {
                            let small =
                                packet_phase_trace(1024, rate, block, packet, phase, ppm, 128);
                            assert_eq!(small, large, "capacity must not retune residency");
                            admitted += 1;
                        } else if target + transfer > 1024 {
                            let (writer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
                            assert!(
                                VirtualPlaybackSource::new(reader, 2, block, rate, BUDGET)
                                    .unwrap()
                                    .with_clock_drift_target(target)
                                    .is_err()
                            );
                            assert!(
                                VirtualMicrophoneSink::new(writer, 2, block, rate, BUDGET)
                                    .unwrap()
                                    .with_clock_drift_target(target)
                                    .is_err()
                            );
                            rejected += 1;
                        } else {
                            // The graph-transfer API does not know the native
                            // packet shape. A packet-phase excursion and the
                            // graph publication can overlap, so the provider
                            // must reserve both, not just their maximum.
                            packet_rejected += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(admitted, 288);
    assert_eq!(rejected, 18);
    assert_eq!(packet_rejected, 99);
}

#[test]
fn graph_target_admission_does_not_claim_native_packet_phase_safety() {
    let (mut writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    writer.write_interleaved_exact(&pcm(584)).unwrap();
    let requested = Arc::new(AtomicU64::new(0));
    let mut sink = VirtualMicrophoneSink::new(writer, 2, 256, 44_100, BUDGET)
        .unwrap()
        .with_clock_drift_target(584)
        .unwrap()
        .with_reset_epoch(requested.clone());
    let mut buffer = CallbackBuffer::<f32>::new(256, 44_100);
    buffer.fill([0.25, -0.5]);
    let mut scratch = [0.0; 240 * 2];
    let mut next_packet = 239_u128;
    for callback in 0..6 {
        let native_time = callback * 256_u128 * 48_000 * 999_800 / (44_100 * 1_000_000);
        while next_packet <= native_time {
            assert_eq!(
                consumer
                    .read_interleaved(&mut scratch)
                    .unwrap()
                    .transferred_frames,
                240
            );
            next_packet += 240;
        }
        let report = buffer.write_sink(&mut sink);
        if callback < 5 {
            assert_eq!(report.transferred_frames, 256);
            assert_eq!(report.xruns, 0);
        } else {
            assert!(report.transferred_frames < 256);
            assert!(report.discontinuity);
            assert_eq!(report.xruns, 1, "unadmitted phase loss must be counted");
            assert_eq!(consumer.available_frames(), 1024);
        }
    }
    assert_eq!(requested.load(Ordering::Acquire), 0);
}

#[test]
fn explicit_content_targets_track_sustained_clock_deficits_in_both_directions() {
    for rate in [44_100, 48_000, 96_000] {
        for ppm in [-500, 500] {
            let trace = packet_phase_trace(1024, rate, 128, 128, 64, ppm, 10_000);
            let settled = &trace[trace.len() - 2000..];
            let playback: i64 = settled.iter().map(|entry| i64::from(entry.2)).sum();
            let microphone: i64 = settled.iter().map(|entry| i64::from(entry.3)).sum();
            assert_eq!(playback.signum(), ppm.signum(), "playback rate={rate}");
            assert_eq!(microphone.signum(), -ppm.signum(), "microphone rate={rate}");
            for sample in settled {
                assert!(sample.2.abs() <= 2_000_000);
                assert!(sample.3.abs() <= 2_000_000);
            }
            let nominal = 10_000_u128 * 128 * 48_000 / u128::from(rate);
            let clock_deficit = nominal * ppm.unsigned_abs() as u128 / 1_000_000;
            let residual_target = 128 + SRC_HEADROOM_FRAMES;
            assert!(clock_deficit > residual_target as u128);
        }
    }
}

#[test]
fn explicit_playback_reprime_preserves_settled_correction_and_exact_silence_edge() {
    const CAPACITY: usize = 1024;
    const TARGET: usize = 384;
    let (mut producer, reader) = audio_bridge(2, CAPACITY, BUDGET).unwrap();
    let content = pcm(CAPACITY);
    let probe = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, 128, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(TARGET)
        .unwrap()
        .with_probe(probe.clone());
    producer.write_interleaved_exact(&content).unwrap();
    let mut buffer = CallbackBuffer::<f32>::new(128, 48_000);
    for _ in 0..300 {
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 128);
        let consumed = CAPACITY - source.carrier.available_frames();
        producer
            .write_interleaved_exact(&content[..consumed * 2])
            .unwrap();
    }
    assert!(source.drift.as_ref().unwrap().servo.correction() > 1000.0);
    while source.carrier.available_frames() > 0 {
        buffer.read_source(&mut source);
    }
    let before = source.drift.as_ref().unwrap().servo.correction();
    assert!(buffer.read_source(&mut source).discontinuity);
    let held = source.drift.as_ref().unwrap().servo.correction();
    assert!(held > 1000.0);
    assert!((held - before).abs() < 100.0);
    assert!(!source.drift.as_ref().unwrap().primed);
    producer
        .write_interleaved_exact(&content[..(TARGET - 1) * 2])
        .unwrap();
    for _ in 0..4 {
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
        buffer.assert_pcm(128);
        assert_eq!(source.carrier.available_frames(), TARGET - 1);
        assert_eq!(source.drift.as_ref().unwrap().servo.correction(), held);
    }
    producer.write_interleaved_exact(&content[..2]).unwrap();
    assert_eq!(buffer.read_source(&mut source).transferred_frames, 128);
    buffer.assert_pcm(SRC_HEADROOM_FRAMES);
    assert!(source.drift.as_ref().unwrap().servo.correction() > 1000.0);
    assert!(probe.snapshot().soft_events >= (SRC_HEADROOM_FRAMES * 2) as u64);
}

#[test]
fn explicit_playback_target_survives_epoch_fencing_and_recovery_edges() {
    const TARGET: usize = 384;
    for rate in [44_100, 48_000, 96_000] {
        let requested = Arc::new(AtomicU64::new(0));
        let producer_ack = Arc::new(AtomicU64::new(0));
        let consumer_ack = Arc::new(AtomicU64::new(0));
        let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
        producer.write_interleaved_exact(&pcm(TARGET)).unwrap();
        let mut source = VirtualPlaybackSource::new(reader, 2, 128, rate, BUDGET)
            .unwrap()
            .with_clock_drift_target(TARGET)
            .unwrap()
            .with_reset_fence(
                requested.clone(),
                producer_ack.clone(),
                consumer_ack.clone(),
            );
        let mut buffer = CallbackBuffer::<f64>::new(128, rate);
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 128);
        buffer.assert_pcm(SRC_HEADROOM_FRAMES);
        let old = source.carrier.available_frames();
        requested.store(1, Ordering::Release);
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
        buffer.assert_pcm(128);
        assert_eq!(source.carrier.available_frames(), old);
        assert_eq!(consumer_ack.load(Ordering::Acquire), 0);
        producer_ack.store(1, Ordering::Release);
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
        assert_eq!(source.carrier.available_frames(), 0);
        assert_eq!(consumer_ack.load(Ordering::Acquire), 1);
        let fresh: Vec<_> = (0..TARGET).flat_map(|_| [-0.125, 0.75]).collect();
        producer
            .write_interleaved_exact(&fresh[..(TARGET - 1) * 2])
            .unwrap();
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 0);
        producer.write_interleaved_exact(&fresh[..2]).unwrap();
        assert_eq!(buffer.read_source(&mut source).transferred_frames, 128);
        buffer.assert_values(SRC_HEADROOM_FRAMES, [-0.125, 0.75]);
        assert_eq!(source.drift.as_ref().unwrap().servo.correction(), 0.0);
        assert_eq!(requested.load(Ordering::Acquire), 1);
    }
}

#[test]
fn explicit_microphone_target_preserves_freshness_and_drop_new_backpressure() {
    for rate in [44_100, 48_000, 96_000] {
        let requested = Arc::new(AtomicU64::new(0));
        let producer_ack = Arc::new(AtomicU64::new(0));
        let consumer_ack = Arc::new(AtomicU64::new(0));
        let (mut writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
        writer.write_interleaved_exact(&pcm(1024)).unwrap();
        let mut sink = VirtualMicrophoneSink::new(writer, 2, 128, rate, BUDGET)
            .unwrap()
            .with_clock_drift_target(384)
            .unwrap()
            .with_reset_fence(
                requested.clone(),
                producer_ack.clone(),
                consumer_ack.clone(),
            );
        let mut buffer = CallbackBuffer::<f64>::new(128, rate);
        buffer.fill([-0.125, 0.75]);
        let report = buffer.write_sink(&mut sink);
        assert_eq!(report.transferred_frames, 0);
        assert_eq!(report.xruns, 1);
        assert_eq!(requested.load(Ordering::Acquire), 0);
        let mut scratch = vec![0.0; 2048];
        assert_eq!(
            consumer
                .read_interleaved(&mut scratch)
                .unwrap()
                .transferred_frames,
            1024
        );
        assert_eq!(scratch, pcm(1024), "overflow keeps the old prefix");
        assert_eq!(buffer.write_sink(&mut sink).transferred_frames, 128);
        requested.store(1, Ordering::Release);
        assert_eq!(buffer.write_sink(&mut sink).transferred_frames, 0);
        assert_eq!(producer_ack.load(Ordering::Acquire), 1);
        assert_eq!(consumer_ack.load(Ordering::Acquire), 0);
        assert!(consumer.discard() > 0);
        buffer.fill([0.5, -0.75]);
        consumer_ack.store(1, Ordering::Release);
        assert_eq!(buffer.write_sink(&mut sink).transferred_frames, 128);
        let report = consumer.read_interleaved(&mut scratch).unwrap();
        assert!(report.transferred_frames > SRC_HEADROOM_FRAMES);
        for frame in scratch[SRC_HEADROOM_FRAMES * 2..report.transferred_frames * 2]
            .as_chunks::<2>()
            .0
        {
            assert!((frame[0] - 0.5).abs() < 1e-6, "no stale pre-epoch content");
            assert!((frame[1] + 0.75).abs() < 1e-6);
        }
        assert_eq!(requested.load(Ordering::Acquire), 1);
    }
}
