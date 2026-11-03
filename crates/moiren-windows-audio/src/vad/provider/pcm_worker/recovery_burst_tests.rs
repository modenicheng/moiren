//! The Engine64/cap8/native480 model is distinct from native qualification.
use super::*;
use crate::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    virtual_owner::{
        BoundaryProbe, ClockSchedule, PreparedMicrophoneReprime, VirtualBufferOptions,
        VirtualMicrophoneSink,
    },
};

const BLOCK: usize = 64;
const PACKET: u32 = 480;
const CAPACITY: usize = 4800;
const TARGET: usize = 1536;
const PERIOD_NS: u64 = 10_000_000;

fn policy() -> PreparedMicrophoneReprime {
    policy_at(TARGET)
}

fn policy_at(target: usize) -> PreparedMicrophoneReprime {
    VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: CAPACITY as u32,
            target_frames: target as u32,
            jitter_frames: 96,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: PACKET,
            max_gap_frames: 512,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(8),
    }
    .prepare_recovery(48_000, BLOCK as u32)
    .unwrap()
    .microphone_reprime(PACKET, CAPACITY as u32)
    .unwrap()
}

fn fixture() -> MicrophoneFixture {
    let mut f = MicrophoneFixture::new(PACKET);
    let (writer, reader) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
    f.writer = writer;
    f.bridge = PcmMicrophoneBridge::new(
        Channels::Stereo,
        SampleFormat::Float32,
        PACKET as usize,
        1 << 20,
        reader,
    )
    .unwrap();
    f.pump = MicrophonePump::prepared(policy());
    f
}

#[test]
fn cap8_reprime_deadline_is_not_renewed_and_expiry_counts_a_full_packet() {
    let policy = policy();
    assert_eq!(policy.required_frames, TARGET as u32);
    assert_eq!(policy.max_wait_100ns, 453_989);
    assert!(!policy.capacity_limited);
    let mut f = fixture();
    let start = f.clock.now();
    f.pump.arm_reprime(start).unwrap();
    let deadline = start + Duration::from_nanos(45_398_900);
    assert_eq!(f.pump.reprime_deadline, Some(deadline));
    for _ in 0..45 {
        f.cycle().unwrap();
        f.wait().unwrap();
        assert!(f.io.written.is_empty());
        assert_eq!(f.pump.reprime_deadline, Some(deadline));
    }
    f.clock.now.set(deadline - Duration::from_nanos(1));
    f.cycle().unwrap();
    assert!(f.io.written.is_empty());
    f.wait().unwrap();
    assert_eq!(f.clock.now(), deadline);
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.io.written[0], vec![0; PACKET as usize * 8]);
    assert_eq!(
        f.state.telemetry.snapshot().silence_frames,
        u64::from(PACKET)
    );
    assert_eq!(f.state.telemetry.snapshot().submission_attempts, 1);
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
fn cap8_reprime_holds_below_target_and_keeps_initial_and_epoch_fences() {
    let mut f = fixture();
    f.pump.arm_reprime(f.clock.now()).unwrap();
    f.writer
        .write_interleaved_exact(&vec![0.5; (TARGET - 1) * 2])
        .unwrap();
    f.cycle().unwrap();
    assert!(f.io.written.is_empty());
    f.writer.write_interleaved_exact(&[0.5; 2]).unwrap();
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.telemetry.snapshot().silence_frames, 0);
    assert!(f.pump.reprime_deadline.is_none());

    let mut initial = fixture();
    initial.cycle().unwrap();
    assert_eq!(initial.io.written.len(), 1, "startup is not reset reprime");
    for cancel in [false, true] {
        let mut f = fixture();
        f.pump.arm_reprime(f.clock.now()).unwrap();
        f.writer
            .write_interleaved_exact(&vec![0.5; TARGET * 2])
            .unwrap();
        if cancel {
            f.state.cancel.store(true, Ordering::Release);
        } else {
            f.state.reset_epoch.store(1, Ordering::Release);
        }
        assert_eq!(f.cycle().unwrap(), PumpState::Reset);
        assert!(f.io.written.is_empty());
        assert_eq!(f.bridge.available_frames(), TARGET);
        assert!(f.pump.reprime_deadline.is_none());
    }
}

fn compiled_burst_matrix<S: moiren_engine::sample::ProcessingSample>(target: usize) {
    for first_wake_ns in [10_666_667, 11_250_000, 11_900_000] {
        for completion_phase_ns in [0, 2_500_000, 9_950_000] {
            for graph_first in [false, true] {
                let state = state();
                let (writer, mut reader) = audio_bridge(2, CAPACITY, 1 << 20).unwrap();
                let probe = Arc::new(BoundaryProbe::default());
                let sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, 48_000, 1 << 20)
                    .unwrap()
                    .with_probe(probe.clone())
                    .with_reset_fence(
                        state.reset_epoch.clone(),
                        state.producer_quiesced_epoch.clone(),
                        state.consumer_quiesced_epoch.clone(),
                    )
                    .with_clock_drift_target(target)
                    .unwrap();
                let mut engine = super::reprime_tests::compiled_microphone::<S>(sink, BLOCK);
                engine.render(BLOCK).unwrap();
                assert!(reader.available_frames() > 0);
                state.reset_epoch.store(1, Ordering::Release);
                engine.render(BLOCK).unwrap();
                let mut observed = 0;
                assert!(quiesce_microphone(&state, &mut observed, &mut reader));
                assert_eq!(observed, 1);
                assert_eq!(reader.available_frames(), 0);
                let mut bridge = PcmMicrophoneBridge::new(
                    Channels::Stereo,
                    SampleFormat::Float32,
                    PACKET as usize,
                    1 << 20,
                    reader,
                )
                .unwrap();
                let mut pump = MicrophonePump::prepared(policy_at(target));
                let mut io = FakeIo::default();
                let mut tickets = vec![];
                let clock = FakeClock::default();
                let start = clock.now();
                pump.arm_reprime(start).unwrap();
                let deadline = pump.reprime_deadline.unwrap();
                let mut schedule =
                    ClockSchedule::with_recovery_cap(1_000_000_000, BLOCK, 48_000, 0, 8).unwrap();
                let mut first_burst = true;
                let mut completion_at = None;
                for t in (0..160_000_000_u64).step_by(50_000) {
                    clock.now.set(start + Duration::from_nanos(t));
                    let mut publish = || {
                        if t < first_wake_ns {
                            return false;
                        }
                        let mut count = 0;
                        while let Some(tick) = schedule.due(t).unwrap() {
                            assert_eq!(tick.clamped_periods, 0);
                            assert_eq!(tick.frames, BLOCK);
                            if first_burst {
                                assert_eq!(tick.backlog_periods, 8 - count);
                            }
                            engine.render(tick.frames).unwrap();
                            count += 1;
                        }
                        if first_burst {
                            assert_eq!(count, 8, "repay eight blocks, not one oversized render");
                            first_burst = false;
                            return true;
                        }
                        false
                    };
                    if graph_first && publish() {
                        assert!(bridge.available_frames() < target);
                    }
                    if completion_at.is_some_and(|due| t >= due) {
                        for ticket in &tickets {
                            let ticket: &PcmTicket = ticket;
                            io.slots[ticket.slot].ready = true;
                        }
                        completion_at = None;
                    }
                    let written = io.written.len();
                    let available = bridge.available_frames();
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
                    if !graph_first && publish() {
                        assert!(bridge.available_frames() < target);
                    }
                    if io.written.len() > written {
                        if written == 0 {
                            assert!(available >= target, "fresh compiled SRC must reach target");
                            assert!(clock.now() < deadline);
                        }
                        completion_at = Some(if t < completion_phase_ns {
                            completion_phase_ns
                        } else {
                            completion_phase_ns
                                + ((t - completion_phase_ns) / PERIOD_NS + 1) * PERIOD_NS
                        });
                    } else if written == 0 {
                        assert_eq!(pump.reprime_deadline, Some(deadline));
                    }
                    assert_eq!(
                        state.telemetry.snapshot().silence_frames,
                        0,
                        "wake={first_wake_ns} completion={completion_phase_ns} graph_first={graph_first} t={t}"
                    );
                }
                assert!(!first_burst);
                assert!(
                    io.written.len() >= 10,
                    "bounded recovery must resume output"
                );
                assert!(io.written.iter().flatten().any(|byte| *byte != 0));
                assert_eq!(probe.snapshot().xruns, 0);
                assert_eq!(state.reset_epoch.load(Ordering::Acquire), 1);
                assert_eq!(state.telemetry.snapshot().driver_dropped_packets, 0);
                assert!(pump.reprime_deadline.is_none());
            }
        }
    }
}

#[test]
fn cap8_reprime_compiled_f32_io_bus_src_packet480_phase_matrix() {
    compiled_burst_matrix::<f32>(TARGET);
}

#[test]
fn cap8_reprime_compiled_f64_io_bus_src_packet480_phase_matrix() {
    compiled_burst_matrix::<f64>(TARGET);
}

#[test]
fn cap8_lower_target1280_compiled_f32_packet480_reprime_phase_matrix() {
    compiled_burst_matrix::<f32>(1280);
}

#[test]
fn cap8_lower_target1280_compiled_f64_packet480_reprime_phase_matrix() {
    compiled_burst_matrix::<f64>(1280);
}
