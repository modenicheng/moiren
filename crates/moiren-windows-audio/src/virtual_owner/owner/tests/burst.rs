use super::*;
use crate::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    virtual_owner::{BoundaryProbe, VirtualBufferOptions, VirtualClockCounters},
};
use moiren_engine::boundary::AudioWriter;

const BLOCK: usize = 64;
const PACKET: usize = 128;
const BUDGET: usize = 1 << 20;

struct BurstClock<'a> {
    now: u64,
    late_at: u64,
    woke: bool,
    packet_before_burst: bool,
    producer: &'a mut AudioWriter<f32>,
    consumer: &'a mut AudioReader<f32>,
    packet: &'a [f32],
    captured: Vec<f32>,
}

// The packet is ordered on either side of one prefilled recovery burst. The
// clock jump is deliberately not a simulation of ongoing native packet cadence.

impl BurstClock<'_> {
    fn native_packet(&mut self) {
        self.producer.write_interleaved_exact(self.packet).unwrap();
        let mut packet = [f32::NAN; PACKET * 2];
        let report = self.consumer.read_interleaved(&mut packet).unwrap();
        assert_eq!(report.transferred_frames, PACKET);
        assert_eq!(report.xruns, 0, "fake native consumer must not underflow");
        self.captured.extend_from_slice(&packet);
    }
}

impl VirtualRealtimeClock for BurstClock<'_> {
    fn frequency(&self) -> u64 {
        48_000
    }

    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        Ok(self.now)
    }

    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        assert!((1..=BLOCK as u64).contains(&ticks));
        if self.woke {
            if !self.packet_before_burst {
                self.native_packet();
            }
            return Ok(ClockWake::Stopped);
        }
        if self.packet_before_burst {
            self.native_packet();
        }
        self.woke = true;
        self.now = self.late_at;
        Ok(ClockWake::Timer)
    }
}

#[derive(Debug, PartialEq)]
struct BurstResult {
    samples: Vec<f32>,
    playback_remaining: usize,
    rendered: VirtualClockCounters,
}

fn burst(cap: u64, capacity: u32, packet_before_burst: bool, omitted: u64) -> BurstResult {
    // Hand-derived bounds: ceil(64 * cap / .998) + FIR 32 + packet 128 + jitter 32.
    let target = match cap {
        1 => 257,
        2 => 321,
        8 => 706,
        _ => unreachable!(),
    };
    let options = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: capacity,
            target_frames: target,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: PACKET as u32,
            max_gap_frames: PACKET as u32,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(cap),
    };
    let budget = options.prepare_recovery(48_000, BLOCK as u32).unwrap();
    assert_eq!(budget.queue.unwrap().prime_frames, target);
    let (mut producer, reader) = audio_bridge(2, capacity as usize, BUDGET).unwrap();
    let (mut writer, mut consumer) = audio_bridge(2, capacity as usize, BUDGET).unwrap();
    let input: Vec<_> = (0..target + PACKET as u32 + 2 * BLOCK as u32)
        .flat_map(|frame| {
            let sample = (frame as f32 * 0.03125).sin() * 0.25;
            [sample, -sample]
        })
        .collect();
    producer
        .write_interleaved_exact(&input[..target as usize * 2])
        .unwrap();
    writer
        .write_interleaved_exact(&vec![0.0; (target as usize + PACKET) * 2])
        .unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let reset = Arc::new(AtomicU64::new(0));
    let source = VirtualPlaybackSource::new(reader, 2, BLOCK, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(target as usize)
        .unwrap()
        .with_probe(playback.clone())
        .with_reset_epoch(reset.clone());
    let sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(target as usize)
        .unwrap()
        .with_probe(microphone.clone())
        .with_reset_epoch(reset.clone());
    let mut graph = LogicalGraph::new();
    let input_node = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let output_node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let bus_input = graph.add_input_port(bus, 2).unwrap();
    graph
        .connect(
            graph.get_node(input_node).unwrap().outputs()[0].id(),
            bus_input,
            SendParams::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(output_node).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let mut bindings = NodeBindings::<f32>::new();
    bindings.bind_source(input_node, source).unwrap();
    bindings.bind_sink(output_node, sink).unwrap();
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
            plan_revision: 1,
            timeline_epoch: 3,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    // Complete the existing startup edge before testing a late wake. This is
    // a deterministic carrier fixture, not the native worker's reprime policy.
    compiled.engine.render(BLOCK).unwrap();
    let queued = capacity as usize - producer.available_frames();
    let refill = target as usize - queued;
    producer
        .write_interleaved_exact(&input[target as usize * 2..(target as usize + refill) * 2])
        .unwrap();
    let surplus = consumer.available_frames() - target as usize;
    consumer
        .read_interleaved(&mut vec![0.0; surplus * 2])
        .unwrap();
    let published = Published::new(&compiled.engine);
    let probe = VirtualClockProbe::new(48_000, BLOCK as u32, cap);
    let mut clock = BurstClock {
        now: 0,
        late_at: (cap + omitted) * BLOCK as u64
            + if packet_before_burst {
                0
            } else {
                BLOCK as u64 / 2
            },
        woke: false,
        packet_before_burst,
        producer: &mut producer,
        consumer: &mut consumer,
        packet: &input[(target as usize + refill) * 2..(target as usize + refill + PACKET) * 2],
        captured: Vec::new(),
    };
    run_with_recovery_cap(
        &mut compiled.engine,
        &mut clock,
        &published,
        std::slice::from_ref(&reset),
        &StopSignal::new().unwrap(),
        &mut ClockTrace::default(),
        cap,
        &probe,
    )
    .unwrap();
    let rendered = probe.snapshot().unwrap().counters;
    assert_eq!(rendered.rendered_periods, cap);
    assert_eq!(rendered.rendered_frames, cap * BLOCK as u64);
    assert_eq!(rendered.clamped_periods, omitted);
    assert_eq!(rendered.clamped_frames, omitted * BLOCK as u64);
    assert_eq!(rendered.late_wake_bursts, u64::from(cap > 1 || omitted > 0));
    assert_eq!(rendered.max_backlog_periods, cap + omitted);
    assert_eq!(rendered.timeline_resets, 0);
    assert_eq!(reset.load(Ordering::Acquire), 0);
    assert_eq!(compiled.engine.timeline(), (cap + 1) * BLOCK as u64);
    for boundary in [&playback, &microphone] {
        let sample = boundary.snapshot();
        assert_eq!(sample.target_frames, Some(target));
        assert_eq!(sample.capacity_frames, capacity);
        assert_eq!(sample.xruns, 0);
        let history = boundary.src_history().unwrap();
        assert_eq!(history.total_callbacks, cap + 1);
        for point in &history.checkpoints {
            assert_eq!(point.transferred_frames, BLOCK as u64);
            assert_eq!(point.xruns, 0);
            assert!(!point.discontinuity);
            assert_eq!(point.after.unwrap().reset_generation, 0);
        }
    }
    let events = playback.snapshot().playback_events.unwrap();
    assert_eq!(events.initial_prime_muted_frames, 32);
    assert_eq!(events.short_read_callbacks, 0);
    assert_eq!(events.recovery_prime_muted_frames, 0);
    assert_eq!(playback.snapshot().soft_events, 32, "startup mute only");
    assert_eq!(microphone.snapshot().soft_events, 0);
    let consumed: u64 = playback
        .src_history()
        .unwrap()
        .checkpoints
        .iter()
        .skip(1)
        .map(|point| point.carrier_frames)
        .sum();
    let produced: u64 = microphone
        .src_history()
        .unwrap()
        .checkpoints
        .iter()
        .skip(1)
        .map(|point| point.carrier_frames)
        .sum();
    let mut samples = std::mem::take(&mut clock.captured);
    drop(clock);
    let mut remaining = vec![f32::NAN; consumer.available_frames() * 2];
    let read = consumer.read_interleaved(&mut remaining).unwrap();
    assert_eq!(read.transferred_frames * 2, remaining.len());
    samples.extend_from_slice(&remaining);
    assert_eq!(samples.len() / 2, target as usize + produced as usize);
    assert_eq!(
        capacity as usize - producer.available_frames(),
        target as usize + PACKET - consumed as usize,
        "every playback carrier frame remains accounted for across packet/burst ordering"
    );
    assert!(samples.iter().all(|sample| sample.is_finite()));
    let burst_samples = &samples[target as usize * 2..];
    assert!(burst_samples.iter().any(|sample| sample.abs() > 0.01));
    assert!(
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| frame[0] == -frame[1])
    );
    let expected = replay_waveform(
        &input,
        &playback.src_history().unwrap().checkpoints,
        &microphone.src_history().unwrap().checkpoints,
        target as usize,
    );
    assert_eq!(samples.len(), expected.len());
    for (index, (&actual, expected)) in samples.iter().zip(expected).enumerate() {
        assert!(
            (f64::from(actual) - expected).abs() < 1e-6,
            "sample {index}: {actual} != {expected}; verify the complete burst, not only retained warmup"
        );
    }
    BurstResult {
        samples,
        playback_remaining: capacity as usize - producer.available_frames(),
        rendered,
    }
}

fn replay_waveform(
    input: &[f32],
    playback: &[crate::virtual_owner::BoundarySrcCheckpoint],
    microphone: &[crate::virtual_owner::BoundarySrcCheckpoint],
    target: usize,
) -> Vec<f64> {
    // Use separate FIRs driven by the known input sequence and observed step,
    // never the graph's output. This tests owner/Bus ordering and unity gain;
    // the resampler's independent numerical oracle lives in phase_tests.
    let mut source = crate::resample::StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let mut sink = crate::resample::StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let mut consumed = 0;
    let mut output = Vec::new();
    let mut warmup = 0;
    for (index, (read, write)) in playback.iter().zip(microphone).enumerate() {
        source.set_correction_ppm((read.processing_step.unwrap() - 1.0) * 1e6);
        for frame in input[consumed * 2..(consumed + read.carrier_frames as usize) * 2]
            .as_chunks::<2>()
            .0
        {
            assert!(source.push_frame([f64::from(frame[0]), f64::from(frame[1])]));
        }
        consumed += read.carrier_frames as usize;
        sink.set_correction_ppm((write.processing_step.unwrap() - 1.0) * 1e6);
        for offset in 0..BLOCK {
            let frame = source.next_frame().unwrap();
            let frame = if offset < read.muted_frames as usize {
                [0.0; 2]
            } else {
                frame.map(|value| f64::from(value as f32))
            };
            assert!(sink.push_frame(frame));
        }
        let before = output.len();
        while let Some(frame) = sink.next_frame() {
            output.extend(frame.map(|value| f64::from(value as f32)));
        }
        assert_eq!((output.len() - before) / 2, write.carrier_frames as usize);
        if index == 0 {
            warmup = output.len() / 2;
        }
    }
    let mut expected = vec![0.0; (target - warmup) * 2];
    expected.extend(output);
    expected
}

#[test]
fn admitted_whole_burst_preserves_both_boundary_targets_independently_of_capacity() {
    for cap in [1, 2, 8] {
        for before in [false, true] {
            assert_eq!(burst(cap, 4096, before, 0), burst(cap, 8192, before, 0));
        }
    }
}

#[test]
fn one_over_cap_wake_counts_omitted_periods_without_resetting_either_boundary() {
    for cap in [1, 2, 8] {
        for before in [false, true] {
            assert_eq!(burst(cap, 4096, before, 3), burst(cap, 8192, before, 3));
        }
    }
}
