//! Hardware-free real ProcessSource/Bus/Pump aggregation, not measured timing.
use super::*;
use moiren_windows_audio::{
    latency::{
        InputDeliveryRecorder, LatencyBudgetError, events::event_history,
        transport::TransportInputPolicy,
    },
    process_loopback::{MeasuredAdmissionError, MeasuredProcessDelivery},
};

fn exercise<S: ProcessingSample>(rate: u32, block: usize) {
    let (mut ingress, source, observed) =
        clock_bridge::capture_bridge(clock_bridge::ClockBridgeConfig {
            output_sample_rate: rate,
            trim_on_prime: false,
            detect_position_gaps: false,
            max_correction_ppm: 0.0,
            ..Default::default()
        })
        .unwrap();
    ingress
        .push_packet(
            &[],
            clock_bridge::CapturePacket {
                frames: 6144,
                flags: stats::SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
        )
        .unwrap();
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let output = graph.create_node(NodeKind::Sink, 2).unwrap();
    let input_port = graph.get_node(input).unwrap().outputs()[0].id();
    moiren_core::graph::edit::connect_to_new_bus_input(
        &mut graph,
        input_port,
        bus,
        SendParams::default(),
    )
    .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(output).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let counters = Arc::new(Counters::default());
    let (tx, rx) = rtrb::RingBuffer::new(8192);
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            input,
            bridge::ProcessSource::<S> {
                source,
                scratch: vec![0.0; 256 * 2].into_boxed_slice(),
                marker: PhantomData,
            },
        )
        .unwrap();
    bindings
        .bind_sink(
            output,
            RingSink::<S> {
                tx,
                counters: counters.clone(),
                marker: PhantomData,
            },
        )
        .unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: f64::from(rate),
                max_block_frames: block,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1024 * 1024,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: u64::from(rate),
        },
    )
    .unwrap();
    let native_info = NativeStreamInfo {
        sample_rate: 48_000,
        format: SampleFormat::Float32,
        buffer_frames: 480,
        channels: 2,
    };
    let mut pump = Pump::new(
        None,
        None,
        Some(rx),
        native_info,
        rate,
        8192,
        counters.clone(),
        true,
    )
    .unwrap();
    let probe = Arc::new(Probe::default());
    let mut native = FakeNative {
        info: native_info,
        failure: Failure::None,
        probe: probe.clone(),
        partial: 73,
    };
    let timeline = Timeline::default();
    let mut wake_ends = [0; 3];
    measured(|| {
        // Seven partial writes finish each 480-frame demand. Partial writes
        // must neither rerender the graph nor create extra consumer events.
        for end in &mut wake_ends {
            for _ in 0..7 {
                assert!(
                    pump.cycle(&mut native, Some(&mut compiled.engine), &timeline)
                        .unwrap()
                );
            }
            *end = compiled.engine.timeline();
        }
    });
    assert_eq!(probe.written.load(Ordering::Relaxed), 3 * 480);
    let wakes = counters.consumer_events.get().unwrap().snapshot().unwrap();
    let reads = observed.queue_history().unwrap();
    assert_eq!(wakes.total_events, 3);
    assert!(reads.total_events > wakes.total_events);
    assert!(
        reads
            .events
            .iter()
            .all(|e| e.sample.requested_frames <= 256)
    );
    let wake_frames = wakes.events.iter().map(|e| e.frames).sum::<u64>();
    let read_frames = reads
        .events
        .iter()
        .map(|e| e.sample.requested_frames)
        .sum::<u64>();
    assert_eq!(wake_frames, compiled.engine.timeline());
    assert_eq!(read_frames, wake_frames);
    assert!(wakes.events.iter().all(|e| e.frames > 256));
    let mut previous_end = 0;
    let mut next_read = 0;
    for (wake, end) in wakes.events.iter().zip(wake_ends) {
        assert_eq!(wake.frames, end - previous_end);
        let count = (wake.frames as usize / block) * block.div_ceil(256);
        let associated = &reads.events[next_read..next_read + count];
        assert_eq!(
            associated
                .iter()
                .map(|e| e.sample.requested_frames)
                .sum::<u64>(),
            wake.frames
        );
        next_read += count;
        previous_end = end;
    }
    assert_eq!(next_read, reads.events.len());
    let prepared = (480 * u64::from(rate)).div_ceil(48_000) + 64 + 2 * block as u64;
    assert!(wakes.events.iter().all(|e| e.frames <= prepared));
    let final_bridge = observed.snapshot();
    assert_eq!(final_bridge.priming_frames, 0);
    assert_eq!(final_bridge.underrun_frames, 0);

    // Use explicitly synthetic clocks only for portable admission arithmetic;
    // the actual Pump frame aggregates above remain unmodified evidence.
    let (mut supply, supply_observer) = event_history();
    let (mut demand, demand_observer) = event_history();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for (index, wake) in wakes.events.iter().enumerate() {
        let stamp = 1_000_000 + index as u64 * 10_000;
        supply.record(48, Some(stamp));
        delivery.packet(48, Some(stamp));
        delivery.callback_burst(48);
        demand.record(wake.frames, Some(stamp + 100));
    }
    let identity = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "synthetic-process.exe".into(),
    };
    let supply_snapshot = supply_observer.snapshot().unwrap();
    let demand_snapshot = demand_observer.snapshot().unwrap();
    let evidence = MeasuredProcessDelivery::from_observations(
        identity.clone(),
        1,
        48_000,
        rate,
        prepared as u32,
        delivery.snapshot(),
        supply_snapshot.clone(),
        demand_snapshot.clone(),
    )
    .unwrap();
    let small = TransportInputPolicy::Target {
        capacity_frames: 8192,
        target_frames: 384,
        jitter_frames: 0,
    };
    // 384 covers a 256-frame segment plus this fixture's 48-frame gap,
    // but cannot cover the old owner's complete prepared native wake.
    assert!(
        ProcessBridgeOptions {
            input: small,
            delivery: Some(
                moiren_windows_audio::latency::transport::InputDeliveryTiming {
                    sample_rate: 48_000,
                    max_packet_frames: 48,
                    max_gap_frames: 48,
                    source:
                        moiren_windows_audio::latency::transport::DeliveryTimingSource::Declared,
                }
            ),
            engine_sample_rate: rate,
            max_demand_frames: 256,
            age_limit_100ns: None,
        }
        .prepare()
        .is_ok()
    );
    assert!(matches!(
        evidence.admit_for_consumer(&identity, 1, small, rate, 256),
        Err(MeasuredAdmissionError::Budget(
            LatencyBudgetError::InsufficientInputBudget
        ))
    ));
    let large = TransportInputPolicy::Target {
        capacity_frames: 8192,
        target_frames: 4096,
        jitter_frames: 0,
    };
    let admitted = evidence
        .admit_for_consumer(&identity, 1, large, rate, 256)
        .unwrap();
    assert_eq!(admitted.max_demand_frames, prepared as u32);

    // Independently preserve observed full wakes even if the caller supplies
    // only a segment-sized prepared/new demand estimate.
    let observed_evidence = MeasuredProcessDelivery::from_observations(
        identity.clone(),
        1,
        48_000,
        rate,
        256,
        delivery.snapshot(),
        supply_snapshot,
        demand_snapshot,
    )
    .unwrap();
    assert!(matches!(
        observed_evidence.admit_for_consumer(&identity, 1, small, rate, 256),
        Err(MeasuredAdmissionError::Budget(
            LatencyBudgetError::InsufficientInputBudget
        ))
    ));
    assert_eq!(
        observed_evidence
            .admit_for_consumer(&identity, 1, large, rate, 256)
            .unwrap()
            .max_demand_frames,
        wakes.events.iter().map(|e| e.frames as u32).max().unwrap()
    );
}

#[test]
fn whole_native_wake_survives_process_read_segmentation_in_both_precisions() {
    for rate in [48_000, 96_000] {
        for block in [64, 512] {
            exercise::<f32>(rate, block);
            exercise::<f64>(rate, block);
        }
    }
}
