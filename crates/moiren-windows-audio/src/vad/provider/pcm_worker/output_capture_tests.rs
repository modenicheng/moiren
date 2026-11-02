use super::*;
use crate::vad::provider::output_capture::{
    CaptureShared, CaptureWorker, VadOutputCaptureArtifact, VadOutputCaptureHandle,
    VadOutputCaptureIdentity,
};

struct Fixture {
    audio: MicrophoneFixture,
    shared: Arc<CaptureShared>,
    handle: VadOutputCaptureHandle,
    capture: Option<CaptureWorker>,
}

impl Fixture {
    fn new(format: SampleFormat, frames: u32, limit: u32) -> Self {
        let mut audio = MicrophoneFixture::new(frames);
        let (writer, reader) = audio_bridge(2, frames as usize * 2, 1 << 20).unwrap();
        audio.writer = writer;
        audio.bridge =
            PcmMicrophoneBridge::new(Channels::Stereo, format, frames as usize, 1 << 20, reader)
                .unwrap();
        audio.tickets.reserve(crate::vad::REQUEST_SLOTS);
        let id = VirtualEndpointId::from_bytes([2; 16]);
        let (shared, handle) = CaptureShared::new(id, limit).unwrap();
        let mut capture = shared.reserve(id).unwrap().unwrap();
        capture.bind(VadOutputCaptureIdentity {
            stable_id: id.to_bytes(),
            native_stamp: [11, 7, 11, 3, 5],
            reset_epoch: 0,
            admission_ticket: audio.state.admitted_ticket.load(Ordering::Acquire),
            active_revision: 9,
            format: format as u32,
            channels: 2,
            packet_frames: frames,
        });
        Self {
            audio,
            shared,
            handle,
            capture: Some(capture),
        }
    }

    fn cycle(&mut self) -> Result<PumpState, BackendError> {
        let f = &mut self.audio;
        microphone_recorded(
            &mut f.io,
            &mut f.bridge,
            &mut f.tickets,
            &mut f.pump,
            (&f.state, 0),
            || f.clock.now(),
            self.capture.as_mut(),
        )
    }

    fn finish(mut self) -> VadOutputCaptureArtifact {
        self.capture.as_mut().unwrap().finished(&Ok(()));
        self.capture.take();
        assert_eq!(self.handle.take_after_join().unwrap_err(), "not_joined");
        self.shared.joined(false);
        self.handle.take_after_join().unwrap()
    }
}

#[test]
fn output_capture_actual_pump_keeps_exact_encoded_bytes_and_zero_fill() {
    for format in [SampleFormat::Pcm16, SampleFormat::Float32] {
        let mut f = Fixture::new(format, 2, 4);
        f.audio
            .writer
            .write_interleaved_exact(&[0.5, -0.5])
            .unwrap();
        assert_eq!(f.cycle().unwrap(), PumpState::Current);
        let expected = f.audio.io.written[0].clone();
        let pending = f.audio.tickets[0];
        f.audio.io.slots[pending.slot].ready = true;
        f.audio.io.slots[pending.slot].stamp.qpc = u64::MAX;
        f.cycle().unwrap();
        let a = f.finish();
        assert_eq!(a.raw, expected);
        assert_eq!(a.stored_frames, 2);
        assert_eq!(a.packets.len(), 1);
        let p = &a.packets[0];
        assert_eq!(p.silence_frames, 1);
        assert_eq!(p.submission, "accepted");
        assert_eq!(p.completion, "completed");
        assert_eq!(p.identity_current_after_submit, Some(true));
        assert_eq!(p.sequence, Some(1));
        assert_eq!(p.host_submission_qpc, Some(u64::MAX));
        assert_eq!(p.completion_bytes, Some(expected.len()));
        assert!(a.raw[a.raw.len() / 2..].iter().all(|&byte| byte == 0));
    }
}

#[test]
fn output_capture_actual_pump_preserves_rejected_and_terminal_completion_facts() {
    for mode in 0..5 {
        let mut f = Fixture::new(SampleFormat::Float32, 2, 8);
        if mode == 0 {
            f.audio.io.quota_writes = 1;
        }
        f.cycle().unwrap();
        if mode != 0 {
            let ticket = f.audio.tickets[0];
            let slot = &mut f.audio.io.slots[ticket.slot];
            slot.ready = true;
            slot.stamp.qpc = u64::MAX;
            match mode {
                1 => slot.dropped = true,
                2 => slot.wrong_shape = true,
                3 => f.audio.io.quota_completions = 1,
                4 => slot.capture = false,
                _ => unreachable!(),
            }
            assert_eq!(f.cycle().is_err(), matches!(mode, 2 | 4));
        }
        let a = f.finish();
        let p = &a.packets[0];
        assert_eq!(
            p.submission,
            if mode == 0 {
                "quota_rejected"
            } else {
                "accepted"
            }
        );
        assert_eq!(
            p.completion,
            match mode {
                0 => "not_submitted",
                1 => "dropped",
                2 | 4 => "wrong_shape",
                3 => "quota_rejected",
                _ => unreachable!(),
            }
        );
        assert_eq!(a.raw.len(), if mode == 3 { 32 } else { 16 });
        if mode == 3 {
            // Existing quota-completion recovery can encode one fresh packet
            // immediately. The witness must not suppress that second attempt.
            assert_eq!(a.packets[1].submission, "accepted");
            assert_eq!(a.packets[1].completion, "retired_completion_unobserved");
        }
    }
}

#[test]
fn output_capture_actual_pump_fences_after_encoding_without_inventing_submission() {
    let mut f = Fixture::new(SampleFormat::Float32, 2, 8);
    f.audio.writer.write_interleaved_exact(&[0.25; 4]).unwrap();
    let calls = Cell::new(0);
    let a = &mut f.audio;
    assert_eq!(
        microphone_recorded(
            &mut a.io,
            &mut a.bridge,
            &mut a.tickets,
            &mut a.pump,
            (&a.state, 0),
            || {
                calls.set(calls.get() + 1);
                // The fourth clock sample is the existing post-encode fence.
                if calls.get() == 4 {
                    a.state.revoke();
                }
                a.clock.now()
            },
            f.capture.as_mut(),
        )
        .unwrap(),
        PumpState::Reset
    );
    assert!(a.io.written.is_empty());
    assert_eq!(a.bridge.available_frames(), 0);
    let artifact = f.finish();
    assert_eq!(artifact.raw, 0.25_f32.to_le_bytes().repeat(4));
    assert_eq!(artifact.packets[0].submission, "fenced");
    assert_eq!(artifact.packets[0].completion, "not_submitted");
    assert_eq!(artifact.packets[0].sequence, None);
}

#[test]
fn output_capture_actual_pump_cap_does_not_change_io_or_telemetry() {
    let mut enabled = Fixture::new(SampleFormat::Float32, 2, 2);
    let mut disabled = Fixture::new(SampleFormat::Float32, 2, 2);
    disabled.capture.take();
    for n in 1..=3 {
        for f in [&mut enabled, &mut disabled] {
            f.audio
                .writer
                .write_interleaved_exact(&[n as f32; 4])
                .unwrap();
            f.audio.clock.advance(Duration::from_micros(50));
            f.cycle().unwrap();
            let pending = f.audio.tickets[0];
            f.audio.io.slots[pending.slot].ready = true;
            f.audio.io.slots[pending.slot].stamp.qpc = u64::MAX;
            f.cycle().unwrap();
        }
    }
    assert_eq!(enabled.audio.io.written, disabled.audio.io.written);
    assert_eq!(enabled.audio.io.collected, disabled.audio.io.collected);
    assert_eq!(
        enabled.audio.bridge.available_frames(),
        disabled.audio.bridge.available_frames()
    );
    let mut left = enabled.audio.state.telemetry_snapshot();
    let mut right = disabled.audio.state.telemetry_snapshot();
    // These wall-clock intervals are sampled independently by existing telemetry.
    left.max_completion_interval_100ns = 0;
    right.max_completion_interval_100ns = 0;
    assert_eq!(left, right);
    let a = enabled.finish();
    assert_eq!(a.packets.len(), 1);
    assert_eq!(a.packets[0].completion, "completed");
    assert_eq!(a.seal_reason, "frame_capacity");
}

struct PreparedIo<'a> {
    book: SlotBook,
    packet: [u8; 32],
    bytes: usize,
    pending: Option<PcmTicket>,
    revoke_on_write: Option<&'a SessionState>,
}

impl PcmIo for PreparedIo<'_> {
    fn wait_activity(&mut self, _: PcmTicket, _: Duration) -> Result<(), ClientError> {
        Ok(())
    }

    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        assert_eq!(self.pending.take(), Some(ticket));
        self.book.finish(ticket).unwrap();
        Ok(Some(PcmCompletion::Capture {
            stamp: PcmPacketStamp {
                sequence: 1,
                qpc: u64::MAX,
            },
            bytes: self.bytes,
        }))
    }

    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        unreachable!("MIC never submits a READ")
    }

    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, ClientError> {
        self.bytes = packet.len();
        self.packet[..self.bytes].copy_from_slice(packet);
        let ticket = self.book.reserve().unwrap();
        self.pending = Some(ticket);
        if let Some(state) = self.revoke_on_write {
            state.revoke();
        }
        Ok(ticket)
    }
}

#[test]
fn output_capture_actual_prepared_pump_does_not_allocate_or_hide_write_race() {
    for revoke in [false, true] {
        let mut f = Fixture::new(SampleFormat::Float32, 2, 8);
        let a = &mut f.audio;
        a.writer.write_interleaved_exact(&[0.125; 4]).unwrap();
        let mut io = PreparedIo {
            book: SlotBook::new().unwrap(),
            packet: [0; 32],
            bytes: 0,
            pending: None,
            revoke_on_write: revoke.then_some(&a.state),
        };
        crate::test_allocation::assert_allocates_nothing(|| {
            microphone_recorded(
                &mut io,
                &mut a.bridge,
                &mut a.tickets,
                &mut a.pump,
                (&a.state, 0),
                || a.clock.now(),
                f.capture.as_mut(),
            )
            .unwrap();
            if !revoke {
                microphone_recorded(
                    &mut io,
                    &mut a.bridge,
                    &mut a.tickets,
                    &mut a.pump,
                    (&a.state, 0),
                    || a.clock.now(),
                    f.capture.as_mut(),
                )
                .unwrap();
            }
        });
        let expected = io.packet[..io.bytes].to_vec();
        let artifact = f.finish();
        assert_eq!(artifact.raw, expected);
        let p = &artifact.packets[0];
        assert_eq!(p.submission, "accepted");
        assert_eq!(p.identity_current_after_submit, Some(!revoke));
        assert_eq!(
            p.completion,
            if revoke {
                "retired_completion_unobserved"
            } else {
                "completed"
            }
        );
    }
}

#[test]
fn output_capture_real_session_join_and_drop_are_required_after_drained_or_panic() {
    for mode in 0..3 {
        let mut f = Fixture::new(SampleFormat::Float32, 2, 8);
        f.cycle().unwrap();
        let mut capture = f.capture.take().unwrap();
        let state = Arc::new(f.audio.state);
        let worker_state = state.clone();
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        let (finish_tx, finish_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            if mode != 2 {
                let result = Err(BackendError::Failed("output witness IO failure".into()));
                capture.finished(&result);
                *worker_state.failure.lock().unwrap() = result.err();
                worker_state.status.store(2, Ordering::Release);
            }
            drop(capture);
            worker_state.drained.store(true, Ordering::Release);
            exited_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            assert_ne!(mode, 2, "output witness test panic");
        });
        let session = VadSession {
            state,
            worker: Some(worker),
            capture: None,
            output_capture: Some(f.shared.clone()),
        };
        exited_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(f.handle.take_after_join().unwrap_err(), "not_joined");
        finish_tx.send(()).unwrap();
        if mode == 1 {
            drop(session);
        } else {
            assert!(Box::new(session).stop_and_join().is_err());
        }
        let a = f.handle.take_after_join().unwrap();
        assert_eq!(a.raw, vec![0; 16]);
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
