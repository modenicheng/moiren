use super::*;
use crate::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    virtual_owner::VirtualBufferOptions,
};

fn configured() -> crate::virtual_owner::PreparedMicrophoneReprime {
    VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 1024,
            target_frames: 369,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 48,
            max_gap_frames: 48,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    }
    .prepare_recovery(48_000, 256)
    .unwrap()
    .microphone_reprime(48, 1024)
    .unwrap()
}

fn fixture() -> MicrophoneFixture {
    let mut f = MicrophoneFixture::new(48);
    let (writer, reader) = audio_bridge(2, 1024, 1 << 20).unwrap();
    f.writer = writer;
    f.bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 48, 1 << 20, reader)
            .unwrap();
    f.pump = MicrophonePump::prepared(configured());
    f
}

#[test]
fn prepared_reprime_does_not_timeout_before_the_next_engine_publication() {
    let mut f = fixture();
    f.pump.arm_reprime(f.clock.now()).unwrap();
    for _ in 0..5 {
        f.clock.advance(Duration::from_millis(1));
        f.cycle().unwrap();
        assert!(f.io.written.is_empty(), "Engine256 has not published yet");
    }
    f.writer.write_interleaved_exact(&vec![0.5; 512]).unwrap();
    f.clock.advance(Duration::from_micros(334));
    f.cycle().unwrap();
    assert!(
        f.io.written.is_empty(),
        "one block is below admitted target369"
    );
    f.writer.write_interleaved_exact(&vec![0.5; 512]).unwrap();
    f.clock.advance(Duration::from_micros(5334));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.telemetry.snapshot().silence_frames, 0);
    assert_eq!(f.io.written[0], 0.5_f32.to_le_bytes().repeat(96));
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
}

#[test]
fn prepared_reprime_uses_one_unrenewed_deadline_and_counts_fallback_silence() {
    let mut f = fixture();
    let start = f.clock.now();
    f.pump.arm_reprime(start).unwrap();
    let deadline = f.pump.reprime_deadline.unwrap();
    for _ in 0..20 {
        f.cycle().unwrap();
        f.wait().unwrap();
        if f.clock.now() >= deadline {
            break;
        }
        assert_eq!(f.pump.reprime_deadline, Some(deadline));
        assert!(f.io.written.is_empty());
    }
    f.clock.now.set(deadline - Duration::from_nanos(1));
    f.cycle().unwrap();
    assert!(f.io.written.is_empty());
    f.clock.advance(Duration::from_nanos(1));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.telemetry.snapshot().silence_frames, 48);
    assert!(f.pump.reprime_deadline.is_none());
    assert!(
        f.clock
            .sleeps
            .borrow()
            .iter()
            .all(|d| *d <= Duration::from_millis(1))
    );
}

#[test]
fn prepared_reprime_preserves_immediate_initial_start_and_cancel_epoch_fences() {
    let mut f = fixture();
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1, "first startup is not a reset");
    for cancel in [false, true] {
        let mut f = fixture();
        f.pump.arm_reprime(f.clock.now()).unwrap();
        f.writer.write_interleaved_exact(&vec![0.5; 1024]).unwrap();
        if cancel {
            f.state.cancel.store(true, Ordering::Release);
        } else {
            f.state.reset_epoch.store(1, Ordering::Release);
        }
        assert_eq!(f.cycle().unwrap(), PumpState::Reset);
        assert!(f.io.written.is_empty());
        assert!(f.pump.reprime_deadline.is_none());
    }
}

fn compiled_microphone<S: moiren_engine::sample::ProcessingSample>(
    sink: crate::virtual_owner::VirtualMicrophoneSink,
) -> Engine<S> {
    use moiren_core::graph::edit::connect_to_new_bus_input;
    use moiren_engine::boundary::ConstantSource;
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let sink_node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let source_port = graph.get_node(source).unwrap().outputs()[0].id();
    connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default()).unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(sink_node).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 2,
                value: 0.5,
            },
        )
        .unwrap();
    bindings.bind_sink(sink_node, sink).unwrap();
    compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 256,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1 << 20,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap()
    .engine
}

fn graph_reset_matrix<S: moiren_engine::sample::ProcessingSample>() {
    use crate::virtual_owner::{BoundaryProbe, VirtualMicrophoneSink};
    for explicit in [false, true] {
        for first_publication_ns in [1, 2_666_667, 5_333_334] {
            for completion_phase_ns in [0, 250_000, 999_000] {
                for graph_first in [false, true] {
                    let state = state();
                    let (writer, mut reader) = audio_bridge(2, 1024, 1 << 20).unwrap();
                    let probe = Arc::new(BoundaryProbe::default());
                    let sink = VirtualMicrophoneSink::new(writer, 2, 256, 48_000, 1 << 20)
                        .unwrap()
                        .with_probe(probe.clone())
                        .with_reset_fence(
                            state.reset_epoch.clone(),
                            state.producer_quiesced_epoch.clone(),
                            state.consumer_quiesced_epoch.clone(),
                        );
                    let sink = if explicit {
                        sink.with_clock_drift_target(369)
                    } else {
                        sink.with_clock_drift()
                    }
                    .unwrap();
                    let mut engine = compiled_microphone::<S>(sink);
                    engine.render(256).unwrap();
                    assert!(reader.available_frames() > 0);
                    state.reset_epoch.store(1, Ordering::Release);
                    // The graph must acknowledge before the consumer discards
                    // old FIR/carrier output. No test-side fresh prefill occurs.
                    engine.render(256).unwrap();
                    let mut observed = 0;
                    assert!(quiesce_microphone(&state, &mut observed, &mut reader));
                    assert_eq!(reader.available_frames(), 0);
                    let policy = if explicit {
                        configured()
                    } else {
                        VirtualBufferOptions {
                            recovery_cap_periods: Some(1),
                            ..Default::default()
                        }
                        .prepare_recovery(48_000, 256)
                        .unwrap()
                        .microphone_reprime(48, 1024)
                        .unwrap()
                    };
                    let mut pump = MicrophonePump::prepared(policy);
                    let mut bridge = PcmMicrophoneBridge::new(
                        Channels::Stereo,
                        SampleFormat::Float32,
                        48,
                        1 << 20,
                        reader,
                    )
                    .unwrap();
                    let mut io = FakeIo::default();
                    let mut tickets = vec![];
                    let clock = FakeClock::default();
                    let start = clock.now();
                    pump.arm_reprime(start).unwrap();
                    let mut publication = first_publication_ns;
                    let mut completion = completion_phase_ns;
                    for t in (0..80_000_000_u64).step_by(50_000) {
                        clock.now.set(start + Duration::from_nanos(t));
                        let publish = t >= publication;
                        if publish && graph_first {
                            engine.render(256).unwrap();
                        }
                        if t >= completion {
                            for ticket in &tickets {
                                let ticket: &PcmTicket = ticket;
                                io.slots[ticket.slot].ready = true;
                            }
                            completion += 1_000_000;
                        }
                        assert_eq!(
                            microphone(
                                &mut io,
                                &mut bridge,
                                &mut tickets,
                                &mut pump,
                                &state,
                                observed,
                                || clock.now(),
                            )
                            .unwrap(),
                            PumpState::Current
                        );
                        if publish && !graph_first {
                            engine.render(256).unwrap();
                        }
                        if publish {
                            publication += 5_333_334;
                        }
                        assert_eq!(
                            state.telemetry.snapshot().silence_frames,
                            0,
                            "explicit={explicit} first={first_publication_ns} phase={completion_phase_ns} graph_first={graph_first} t={t}"
                        );
                    }
                    assert!(
                        io.written.len() > 50,
                        "continuing output, not an infinite hold"
                    );
                    assert!(io.written.iter().flatten().any(|byte| *byte != 0));
                    assert_eq!(probe.snapshot().xruns, 0, "no hidden carrier overflow");
                    assert_eq!(state.reset_epoch.load(Ordering::Acquire), 1);
                }
            }
        }
    }
}

#[test]
fn compiled_io_bus_microphone_src_reprime_f32_publication_phase_matrix() {
    graph_reset_matrix::<f32>();
}

#[test]
fn compiled_io_bus_microphone_src_reprime_f64_publication_phase_matrix() {
    graph_reset_matrix::<f64>();
}
