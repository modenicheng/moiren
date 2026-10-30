use super::*;
use crate::vad::provider::input_capture::{CaptureShared, VadInputCaptureIdentity};

fn capture_identity(state: &SessionState, format: SampleFormat) -> VadInputCaptureIdentity {
    let ticket = state.admission.set(Some(9));
    state.admitted_ticket.store(ticket, Ordering::Release);
    state.admitted_revision.store(9, Ordering::Release);
    state.active.store(true, Ordering::Release);
    VadInputCaptureIdentity {
        stable_id: [1; 16],
        native_stamp: [11, 7, 11, 3, 5],
        reset_epoch: 0,
        admission_ticket: ticket,
        active_revision: 9,
        format: format as u32,
        channels: 2,
        packet_frames: 1,
    }
}

#[test]
fn input_capture_actual_pump_retains_raw_before_decode_without_changing_carrier() {
    for (format, raw) in [
        (
            SampleFormat::Pcm16,
            [16384_i16.to_le_bytes(), (-16384_i16).to_le_bytes()].concat(),
        ),
        (
            SampleFormat::Float32,
            [0.5_f32.to_le_bytes(), (-0.5_f32).to_le_bytes()].concat(),
        ),
    ] {
        let state = state();
        let identity = capture_identity(&state, format);
        let id = VirtualEndpointId::from_bytes(identity.stable_id);
        let (shared, handle) = CaptureShared::new(id, 4).unwrap();
        let mut capture = shared.reserve(id).unwrap().unwrap();
        capture.bind(identity);
        let mut io = FakeIo::default();
        let mut tickets = pending_reads(&mut io);
        io.slots[tickets[0].slot].packet = raw.clone();
        io.slots[tickets[0].slot].ready = true;
        let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
        let mut bridge = PcmPlaybackBridge::new(Channels::Stereo, format, 1, 4096, writer).unwrap();
        playback_recorded(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            0,
            Some(&mut capture),
        )
        .unwrap();
        let mut samples = [0.; 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples, [0.5, -0.5]);
        assert_eq!(io.reads, 5);
        drop(capture);
        assert_eq!(handle.take_after_join().unwrap_err(), "not_joined");
        shared.joined(false);
        let artifact = handle.take_after_join().unwrap();
        assert_eq!(artifact.raw, raw);
        assert_eq!(artifact.packets[0].publication, "published");
    }
}

struct Fixture {
    state: SessionState,
    shared: Arc<CaptureShared>,
    handle: crate::vad::VadInputCaptureHandle,
    capture: Option<crate::vad::provider::input_capture::CaptureWorker>,
    io: FakeIo,
    tickets: Vec<PcmTicket>,
    bridge: PcmPlaybackBridge,
    reader: AudioReader<f32>,
    order: PacketOrder,
}

impl Fixture {
    fn new(frames: u32, capacity: usize) -> Self {
        let state = state();
        let identity = capture_identity(&state, SampleFormat::Float32);
        let id = VirtualEndpointId::from_bytes(identity.stable_id);
        let (shared, handle) = CaptureShared::new(id, frames).unwrap();
        let mut capture = shared.reserve(id).unwrap().unwrap();
        capture.bind(identity);
        let mut io = FakeIo::default();
        let tickets = pending_reads(&mut io);
        let (writer, reader) = audio_bridge(2, capacity, 4096).unwrap();
        Self {
            state,
            shared,
            handle,
            capture: Some(capture),
            io,
            tickets,
            bridge: PcmPlaybackBridge::new(
                Channels::Stereo,
                SampleFormat::Float32,
                1,
                4096,
                writer,
            )
            .unwrap(),
            reader,
            order: PacketOrder::default(),
        }
    }

    fn packet(&mut self, sequence: u64, samples: [f32; 2]) {
        let slot = &mut self.io.slots[self.tickets[0].slot];
        slot.stamp = PcmPacketStamp {
            sequence,
            qpc: sequence,
        };
        slot.packet = samples.into_iter().flat_map(f32::to_le_bytes).collect();
        slot.ready = true;
    }

    fn pump(&mut self) -> Result<PumpState, BackendError> {
        playback_recorded(
            &mut self.io,
            &mut self.bridge,
            &mut self.tickets,
            &mut self.order,
            &self.state,
            0,
            self.capture.as_mut(),
        )
    }

    fn finish(mut self) -> crate::vad::VadInputCaptureArtifact {
        self.capture.take();
        self.shared.joined(false);
        self.handle.take_after_join().unwrap()
    }
}

#[test]
fn input_capture_preserves_zero_nonfinite_bits_and_excludes_stale() {
    let mut f = Fixture::new(4, 8);
    let nan = f32::from_bits(0x7fc01234);
    f.packet(1, [0., -0.]);
    f.pump().unwrap();
    f.packet(1, [5., 5.]);
    f.pump().unwrap();
    f.packet(2, [nan, f32::INFINITY]);
    f.pump().unwrap();
    let a = f.finish();
    assert_eq!(
        a.raw,
        [0_f32, -0., nan, f32::INFINITY]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(a.excluded_stale_packets, 1);
    assert_eq!(a.packets.len(), 2);
}

#[test]
fn input_capture_capacity_is_not_audio_drop_and_records_carrier_failure() {
    let mut f = Fixture::new(1, 8);
    for n in 1..=3 {
        f.packet(n, [n as f32; 2]);
        assert_eq!(f.pump().unwrap(), PumpState::Current);
    }
    let mut samples = [0.; 6];
    f.reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [1., 1., 2., 2., 3., 3.]);
    assert_eq!(f.state.telemetry_snapshot().carrier_dropped_packets, 0);
    let a = f.finish();
    assert_eq!(a.stored_frames, 1);
    assert_eq!(a.seal_reason, "frame_capacity");

    let mut f = Fixture::new(8, 1);
    f.packet(1, [1.; 2]);
    f.pump().unwrap();
    f.packet(2, [2.; 2]);
    assert_eq!(f.pump().unwrap(), PumpState::Reset);
    let a = f.finish();
    assert_eq!(a.packets[1].publication, "carrier_full");
    assert_eq!(a.stored_frames, 2);
}

#[test]
fn input_capture_reset_and_revision_cannot_relabel_old_owner() {
    for reset in [false, true] {
        let mut f = Fixture::new(8, 8);
        f.packet(1, [1.; 2]);
        f.pump().unwrap();
        if reset {
            f.state.reset_epoch.store(1, Ordering::Release);
        } else {
            let ticket = f.state.admission.set(Some(10));
            f.state.admitted_ticket.store(ticket, Ordering::Release);
            f.state.admitted_revision.store(10, Ordering::Release);
        }
        f.packet(2, [2.; 2]);
        let _ = f.pump().unwrap();
        let a = f.finish();
        assert_eq!(a.stored_frames, 1);
        assert_eq!(a.identity.unwrap().active_revision, 9);
        assert_eq!(a.seal_reason, "identity_changed");
    }
}

#[test]
fn input_capture_short_packet_and_early_error_survive_until_real_join() {
    let mut f = Fixture::new(8, 8);
    f.packet(1, [1.; 2]);
    f.io.slots[f.tickets[0].slot].packet.truncate(4);
    assert!(f.pump().is_err());
    let a = f.finish();
    assert_eq!(a.raw, 1_f32.to_le_bytes());
    assert_eq!(a.packets[0].publication, "decode_error");

    let f = Fixture::new(8, 8);
    assert!(
        f.shared
            .reserve(VirtualEndpointId::from_bytes([1; 16]))
            .unwrap()
            .is_none()
    );
    assert_eq!(f.handle.take_after_join().unwrap_err(), "not_joined");
    let a = f.finish();
    assert!(a.identity.is_none());
    assert!(a.raw.is_empty());
}

#[test]
fn input_capture_empty_capacity_no_data_and_spawn_failure_stay_unknown() {
    let mut f = Fixture::new(1, 8);
    let mut identity = capture_identity(&f.state, SampleFormat::Float32);
    identity.packet_frames = 2;
    f.capture.as_mut().unwrap().bind(identity);
    f.packet(1, [1.; 2]);
    f.io.slots[f.tickets[0].slot]
        .packet
        .extend_from_slice(&[0; 8]);
    assert!(f.pump().is_err());
    let a = f.finish();
    assert!(a.raw.is_empty());
    assert!(a.identity.is_none());
    assert_eq!(a.seal_reason, "capacity_whole_packet");

    let id = VirtualEndpointId::from_bytes([4; 16]);
    assert!(CaptureShared::new(id, 0).is_err());
    assert!(CaptureShared::new(id, 480_001).is_err());
    let (shared, handle) = CaptureShared::new(id, 1).unwrap();
    assert_eq!(handle.take_after_join().unwrap_err(), "not_started_unknown");
    assert!(
        shared
            .reserve(VirtualEndpointId::from_bytes([5; 16]))
            .unwrap()
            .is_none()
    );
    let capture = shared.reserve(id).unwrap().unwrap();
    drop(capture);
    shared.spawn_failed();
    assert_eq!(
        handle.take_after_join().unwrap_err(),
        "spawn_failed_no_worker_unknown"
    );
}

#[test]
fn input_capture_session_join_and_drop_retain_error_and_panic_without_early_take() {
    for mode in 0..3 {
        let mut f = Fixture::new(8, 8);
        f.packet(1, [0.; 2]);
        f.pump().unwrap();
        let capture = f.capture.take().unwrap();
        let state = Arc::new(f.state);
        let worker_state = state.clone();
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        let (finish_tx, finish_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut capture = capture;
            if mode == 2 {
                drop(capture);
                worker_state.drained.store(true, Ordering::Release);
                exited_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                panic!("diagnostic test worker panic");
            }
            let result = Err(BackendError::Failed("early IO failure".into()));
            capture.finished(&result);
            *worker_state.failure.lock().unwrap() = result.err();
            worker_state.status.store(2, Ordering::Release);
            drop(capture);
            worker_state.drained.store(true, Ordering::Release);
            exited_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
        });
        let session = VadSession {
            state: state.clone(),
            worker: Some(worker),
            capture: Some(f.shared.clone()),
        };
        exited_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(state.drained.load(Ordering::Acquire));
        assert_eq!(f.handle.take_after_join().unwrap_err(), "not_joined");
        finish_tx.send(()).unwrap();
        if mode == 1 {
            drop(session);
        } else {
            assert!(Box::new(session).stop_and_join().is_err());
        }
        let a = f.handle.take_after_join().unwrap();
        assert_eq!(a.stored_frames, 1);
        assert_eq!(
            a.worker_join,
            if mode == 2 {
                "joined_panicked"
            } else {
                "joined"
            }
        );
        assert_eq!(
            a.worker_outcome,
            if mode == 2 { "unknown" } else { "failed" }
        );
        assert_eq!(f.handle.take_after_join().unwrap_err(), "already_taken");
    }
}

#[test]
fn input_capture_enabled_disabled_actual_pump_have_identical_io_carrier_and_telemetry() {
    let mut enabled = Fixture::new(2, 8);
    let mut disabled = Fixture::new(2, 8);
    disabled.capture.take();
    for sequence in [1, 1, 4, 5] {
        for f in [&mut enabled, &mut disabled] {
            f.packet(sequence, [sequence as f32; 2]);
            // Deterministic unavailable-delay evidence, not two wall-clock
            // samples mistaken for equal measured collection latency.
            f.io.slots[f.tickets[0].slot].stamp.qpc = u64::MAX;
            assert_eq!(f.pump().unwrap(), PumpState::Current);
            wait_for_activity(&mut f.io, &f.tickets, &f.state, 0).unwrap();
        }
    }
    let mut left = [0.; 6];
    let mut right = [0.; 6];
    enabled.reader.read_interleaved(&mut left).unwrap();
    disabled.reader.read_interleaved(&mut right).unwrap();
    assert_eq!(left, right);
    assert_eq!(enabled.io.reads, disabled.io.reads);
    assert_eq!(enabled.io.collected, disabled.io.collected);
    assert_eq!(
        enabled
            .io
            .waits
            .iter()
            .map(|(t, d)| (t.slot, *d))
            .collect::<Vec<_>>(),
        disabled
            .io
            .waits
            .iter()
            .map(|(t, d)| (t.slot, *d))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        enabled.tickets.iter().map(|t| t.slot).collect::<Vec<_>>(),
        disabled.tickets.iter().map(|t| t.slot).collect::<Vec<_>>()
    );
    let mut left = enabled.state.telemetry_snapshot();
    let mut right = disabled.state.telemetry_snapshot();
    // Existing completion timing samples Instant independently in each run.
    left.max_completion_interval_100ns = 0;
    right.max_completion_interval_100ns = 0;
    assert_eq!(left, right);
}

#[test]
fn input_capture_pre_first_packet_reset_can_arm_fresh_but_later_reset_seals() {
    let mut f = Fixture::new(8, 8);
    f.state.reset_epoch.store(1, Ordering::Release);
    assert_eq!(f.pump().unwrap(), PumpState::Reset);
    let mut identity = capture_identity(&f.state, SampleFormat::Float32);
    identity.native_stamp[3] += 1;
    identity.reset_epoch = 1;
    f.capture.as_mut().unwrap().bind(identity);
    f.state.producer_quiesced_epoch.store(1, Ordering::Release);
    f.state.consumer_quiesced_epoch.store(1, Ordering::Release);
    f.packet(1, [1.; 2]);
    playback_recorded(
        &mut f.io,
        &mut f.bridge,
        &mut f.tickets,
        &mut f.order,
        &f.state,
        1,
        f.capture.as_mut(),
    )
    .unwrap();
    f.state.reset_epoch.store(2, Ordering::Release);
    assert_eq!(
        playback_recorded(
            &mut f.io,
            &mut f.bridge,
            &mut f.tickets,
            &mut f.order,
            &f.state,
            1,
            f.capture.as_mut()
        )
        .unwrap(),
        PumpState::Reset
    );
    let a = f.finish();
    assert_eq!(a.stored_frames, 1);
    assert_eq!(a.identity.unwrap(), identity);
    assert_eq!(a.seal_reason, "identity_changed");
}
