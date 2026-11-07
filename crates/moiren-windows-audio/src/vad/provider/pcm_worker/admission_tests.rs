//! Revocation stops new PCM submissions without erasing already pending IO.
use super::*;

fn admit(state: &SessionState) {
    state.active.store(true, Ordering::Release);
    let ticket = state.admission.set(Some(9));
    state.admitted_ticket.store(ticket, Ordering::Release);
    state.admitted_revision.store(9, Ordering::Release);
    assert!(state.admitted());
}

struct RevokeAfterRead<'a> {
    inner: FakeIo,
    state: &'a SessionState,
}

impl PcmIo for RevokeAfterRead<'_> {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), ClientError> {
        self.inner.wait_activity(ticket, timeout)
    }

    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.inner.complete(ticket)
    }

    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        let ticket = self.inner.read()?;
        self.state.revoke();
        Ok(ticket)
    }

    fn write(&mut self, _: &[u8]) -> Result<PcmTicket, ClientError> {
        panic!("playback must not WRITE")
    }
}

#[test]
fn playback_revoked_before_refill_preserves_pending_peer_without_new_reads() {
    let state = state();
    admit(&state);
    let mut io = FakeIo::default();
    let peer = io.read().unwrap();
    let mut tickets = vec![peer];
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    state.revoke();
    assert_eq!(
        playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            0
        )
        .unwrap(),
        PumpState::Reset
    );
    assert_eq!(io.reads, 1);
    assert_eq!(tickets, [peer]);
    assert!(
        io.book.check(peer).is_ok(),
        "pending native IO still needs drain"
    );
    let counters = state.telemetry_snapshot();
    assert_eq!(counters.submission_attempts, 0);
    assert_eq!(counters.submitted_packets, 0);
    assert_eq!(counters.epoch_resets, 0);
    assert_eq!(counters.recovery_events, 0);
    assert!(!state.drained.load(Ordering::Acquire));
}

#[test]
fn playback_revoked_during_refill_stops_after_the_accepted_read() {
    let state = state();
    admit(&state);
    let mut io = RevokeAfterRead {
        inner: FakeIo::default(),
        state: &state,
    };
    let mut tickets = vec![];
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    assert_eq!(
        playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            0
        )
        .unwrap(),
        PumpState::Reset
    );
    assert_eq!(io.inner.reads, 1);
    assert_eq!(tickets.len(), 1);
    assert!(io.inner.book.check(tickets[0]).is_ok());
    let counters = state.telemetry_snapshot();
    assert_eq!(counters.submission_attempts, 1);
    assert_eq!(counters.submitted_packets, 1);
    assert_eq!(counters.epoch_resets, 0);
    assert_eq!(counters.recovery_events, 0);
    assert!(!state.drained.load(Ordering::Acquire));
}

#[test]
fn microphone_revoked_before_encoding_keeps_carrier_and_does_not_write() {
    let mut f = MicrophoneFixture::new(48);
    admit(&f.state);
    f.writer.write_interleaved_exact(&vec![0.25; 96]).unwrap();
    let queued = f.bridge.available_frames();
    f.state.revoke();
    assert_eq!(f.cycle().unwrap(), PumpState::Reset);
    assert_eq!(f.bridge.available_frames(), queued);
    assert!(f.io.written.is_empty());
    assert!(f.tickets.is_empty());
    let counters = f.state.telemetry_snapshot();
    assert_eq!(counters.submission_attempts, 0);
    assert_eq!(counters.carrier_dropped_packets, 0);
    assert_eq!(counters.epoch_resets, 0);
    assert_eq!(counters.recovery_events, 0);
}

#[test]
fn microphone_revoked_after_encoding_counts_consumed_packet_without_write() {
    let mut f = MicrophoneFixture::new(48);
    admit(&f.state);
    f.writer.write_interleaved_exact(&vec![0.25; 96]).unwrap();
    let initial = f.bridge.available_frames();
    let mut clocks = 0;
    let result = microphone(
        &mut f.io,
        &mut f.bridge,
        &mut f.tickets,
        &mut f.pump,
        &f.state,
        0,
        || {
            clocks += 1;
            if clocks == 4 {
                f.state.revoke();
            }
            f.clock.now()
        },
    )
    .unwrap();
    assert_eq!(result, PumpState::Reset);
    assert!(clocks >= 4, "revocation injected after encoding");
    assert!(f.bridge.available_frames() < initial);
    assert!(f.io.written.is_empty());
    assert!(f.tickets.is_empty());
    let counters = f.state.telemetry_snapshot();
    assert_eq!(counters.submission_attempts, 1);
    assert_eq!(counters.submitted_packets, 0);
    assert_eq!(counters.carrier_dropped_packets, 1);
    assert_eq!(counters.epoch_resets, 0);
    assert_eq!(counters.recovery_events, 0);
}

#[test]
fn revoked_playback_collects_ready_pcm_without_refill_or_erasing_publication() {
    let state = state();
    admit(&state);
    let mut io = FakeIo::default();
    let ticket = io.read().unwrap();
    io.slots[ticket.slot].ready = true;
    let mut tickets = vec![ticket];
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    state.revoke();
    assert_eq!(
        playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            0
        )
        .unwrap(),
        PumpState::Reset
    );
    assert!(tickets.is_empty());
    assert_eq!(io.reads, 1);
    assert_eq!(io.collected, [1]);
    let mut samples = [0.; 2];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(samples, [1.; 2]);
    let counters = state.telemetry_snapshot();
    assert_eq!(counters.completed_packets, 1);
    assert_eq!(counters.carrier_dropped_packets, 0);
}

struct TerminalIo {
    book: SlotBook,
    code: windows::core::HRESULT,
}

impl PcmIo for TerminalIo {
    fn wait_activity(&mut self, _: PcmTicket, _: Duration) -> Result<(), ClientError> {
        panic!("no wait after terminal fault")
    }
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.book.finish(ticket).unwrap();
        Err(ClientError::windows(
            "GetOverlappedResult(VAD PCM)",
            windows::core::Error::from_hresult(self.code),
        ))
    }
    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        panic!("no READ after terminal fault")
    }
    fn write(&mut self, _: &[u8]) -> Result<PcmTicket, ClientError> {
        panic!("no WRITE after terminal fault")
    }
}

#[test]
fn revoked_admission_does_not_hide_terminal_read_or_write_faults_or_peer_drain() {
    use windows::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_REVISION_MISMATCH};
    for code in [ERROR_DEVICE_NOT_CONNECTED, ERROR_REVISION_MISMATCH] {
        for microphone_direction in [false, true] {
            let mut f = MicrophoneFixture::new(48);
            admit(&f.state);
            let mut io = TerminalIo {
                book: SlotBook::new().unwrap(),
                code: code.to_hresult(),
            };
            let first = io.book.reserve().unwrap();
            let peer = io.book.reserve().unwrap();
            let mut tickets = vec![first, peer];
            f.state.revoke();
            let error = if microphone_direction {
                microphone(
                    &mut io,
                    &mut f.bridge,
                    &mut tickets,
                    &mut f.pump,
                    &f.state,
                    0,
                    || f.clock.now(),
                )
            } else {
                let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
                let mut bridge = PcmPlaybackBridge::new(
                    Channels::Stereo,
                    SampleFormat::Float32,
                    1,
                    4096,
                    writer,
                )
                .unwrap();
                playback(
                    &mut io,
                    &mut bridge,
                    &mut tickets,
                    &mut PacketOrder::default(),
                    &f.state,
                    0,
                )
            }
            .unwrap_err();
            let expected = pcm_error(ClientError::windows(
                "GetOverlappedResult(VAD PCM)",
                windows::core::Error::from_hresult(code.to_hresult()),
            ));
            assert_eq!(error, expected);
            assert!(io.book.check(first).is_err());
            assert!(io.book.check(peer).is_ok());
            assert_eq!(io.book.pending(), 1);
            assert!(!f.state.drained.load(Ordering::Acquire));
            assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
            assert_eq!(f.state.telemetry_snapshot().recovery_events, 0);
        }
    }
}

#[test]
fn revoked_admission_skips_native_wait_and_phase_sleep_without_releasing_tickets() {
    let mut f = MicrophoneFixture::new(48);
    admit(&f.state);
    f.cycle().unwrap();
    let pending = f.tickets[0];
    f.state.revoke();
    f.wait().unwrap();
    wait_for_activity(&mut f.io, &f.tickets, &f.state, 0).unwrap();
    assert!(f.io.waits.is_empty());
    assert!(f.clock.sleeps.borrow().is_empty());
    assert!(f.io.book.check(pending).is_ok());
    assert_eq!(f.tickets, [pending]);
    f.io.drain(&mut f.tickets);
    f.pump.next_submit = Some(f.clock.now() + Duration::from_millis(1));
    f.wait().unwrap();
    assert!(f.clock.sleeps.borrow().is_empty());
    assert!(!f.state.drained.load(Ordering::Acquire));
}

#[test]
fn revoked_pump_checks_do_not_allocate_or_free() {
    let mut f = MicrophoneFixture::new(48);
    admit(&f.state);
    f.state.revoke();
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    let result = crate::test_allocation::assert_allocates_nothing(|| {
        playback(
            &mut f.io,
            &mut bridge,
            &mut f.tickets,
            &mut order,
            &f.state,
            0,
        )
    });
    assert_eq!(result.unwrap(), PumpState::Reset);
    let result = crate::test_allocation::assert_allocates_nothing(|| f.cycle());
    assert_eq!(result.unwrap(), PumpState::Reset);
    assert!(f.io.written.is_empty());
}

struct RevokeOnSubmit<'a> {
    state: &'a SessionState,
    code: windows::core::HRESULT,
    calls: usize,
}

impl PcmIo for RevokeOnSubmit<'_> {
    fn wait_activity(&mut self, _: PcmTicket, _: Duration) -> Result<(), ClientError> {
        panic!("submission failure must not wait")
    }
    fn complete(&mut self, _: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        panic!("no request exists before failed submission")
    }
    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        self.calls += 1;
        self.state.revoke();
        Err(ClientError::windows(
            "DeviceIoControl(VAD PCM)",
            windows::core::Error::from_hresult(self.code),
        ))
    }
    fn write(&mut self, _: &[u8]) -> Result<PcmTicket, ClientError> {
        self.read()
    }
}

#[test]
fn revocation_inside_submission_preserves_read_and_write_terminal_errors() {
    use windows::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_REVISION_MISMATCH};
    for code in [ERROR_DEVICE_NOT_CONNECTED, ERROR_REVISION_MISMATCH] {
        for microphone_direction in [false, true] {
            let mut f = MicrophoneFixture::new(48);
            admit(&f.state);
            let mut io = RevokeOnSubmit {
                state: &f.state,
                code: code.to_hresult(),
                calls: 0,
            };
            let error = if microphone_direction {
                microphone(
                    &mut io,
                    &mut f.bridge,
                    &mut f.tickets,
                    &mut f.pump,
                    &f.state,
                    0,
                    || f.clock.now(),
                )
            } else {
                let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
                let mut bridge = PcmPlaybackBridge::new(
                    Channels::Stereo,
                    SampleFormat::Float32,
                    1,
                    4096,
                    writer,
                )
                .unwrap();
                playback(
                    &mut io,
                    &mut bridge,
                    &mut f.tickets,
                    &mut PacketOrder::default(),
                    &f.state,
                    0,
                )
            }
            .unwrap_err();
            assert_eq!(
                error,
                pcm_error(ClientError::windows(
                    "DeviceIoControl(VAD PCM)",
                    windows::core::Error::from_hresult(code.to_hresult())
                ))
            );
            assert_eq!(io.calls, 1);
            assert!(!f.state.admitted());
            assert!(f.tickets.is_empty());
            assert!(!f.state.drained.load(Ordering::Acquire));
            let counters = f.state.telemetry_snapshot();
            assert_eq!(counters.submission_attempts, 1);
            assert_eq!(counters.submitted_packets, 0);
            assert_eq!(counters.recovery_events, 0);
            assert_eq!(counters.epoch_resets, 0);
        }
    }
}

#[test]
fn playback_refill_and_microphone_encode_require_all_session_authority_checks() {
    for change in 0..6 {
        let mut f = MicrophoneFixture::new(48);
        admit(&f.state);
        f.writer.write_interleaved_exact(&vec![0.25; 96]).unwrap();
        let queued = f.bridge.available_frames();
        match change {
            0 => f.state.revoke(),
            1 => f.state.cancel.store(true, Ordering::Release),
            2 => f.state.active.store(false, Ordering::Release),
            3 => f.state.drained.store(true, Ordering::Release),
            4 => {
                f.state.admission.set(Some(9));
            }
            _ => {
                f.state.admission.set(Some(10));
            }
        }
        assert!(!f.state.admitted());
        assert_eq!(f.cycle().unwrap(), PumpState::Reset);
        assert_eq!(f.bridge.available_frames(), queued);
        assert!(f.io.written.is_empty());
        let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
        let mut bridge =
            PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer)
                .unwrap();
        assert_eq!(
            playback(
                &mut f.io,
                &mut bridge,
                &mut f.tickets,
                &mut PacketOrder::default(),
                &f.state,
                0
            )
            .unwrap(),
            PumpState::Reset
        );
        assert_eq!(f.io.reads, 0);
        let counters = f.state.telemetry_snapshot();
        assert_eq!(counters.submission_attempts, 0);
        assert_eq!(counters.carrier_dropped_packets, 0);
        assert_eq!(counters.recovery_events, 0);
        assert_eq!(counters.epoch_resets, 0);
    }
}
