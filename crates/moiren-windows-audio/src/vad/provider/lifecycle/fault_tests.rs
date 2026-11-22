//! Model a native epoch change after QUERY, before collecting old PCM.
//! This is a diagnostic interleaving, not attribution of the desktop fault.
use super::*;
use crate::vad::{PcmCompletion, PcmTicket, SampleFormat, pcm_slots::SlotBook};
use pcm_worker::PcmIo;

struct EpochFaultIo<'a> {
    control: &'a mut Fake,
    book: SlotBook,
    completes: usize,
}

impl PcmIo for EpochFaultIo<'_> {
    fn wait_activity(&mut self, _: PcmTicket, _: Duration) -> Result<(), ClientError> {
        panic!("a terminal completion must not return to waiting")
    }

    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.completes += 1;
        self.book.finish(ticket).unwrap();
        // The native epoch may change without a stream-generation change or
        // a closed gate. Simulate its rejection of this old queued request.
        self.control.current = snapshot_native(
            PdoState::Present,
            3,
            6,
            true,
            11,
            (StreamState::Paused, true, true),
        );
        Err(ClientError::windows(
            "GetOverlappedResult(VAD PCM)",
            windows::core::Error::from_hresult(
                windows::Win32::Foundation::ERROR_REVISION_MISMATCH.to_hresult(),
            ),
        ))
    }

    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        panic!("do not refill after a terminal completion")
    }

    fn write(&mut self, _: &[u8]) -> Result<PcmTicket, ClientError> {
        panic!("playback must not WRITE")
    }
}

#[test]
fn epoch_change_between_query_and_pcm_completion_preserves_fault_and_peer_drain() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    session
        .admitted_ticket
        .store(admission.set(Some(9)), Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    let previous = snapshot(PdoState::Present, 3, 5, true);
    let mut control = Fake {
        current: previous,
        ..Fake::default()
    };
    let (queried, changed) =
        transport::poll_with_started_transport(&mut control, previous, &session, true).unwrap();
    assert!(!changed);
    assert!(session.admitted());
    assert_eq!(queried.gate_epoch(), 5);

    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let error = {
        let mut io = EpochFaultIo {
            control: &mut control,
            book: SlotBook::new().unwrap(),
            completes: 0,
        };
        let first = io.book.reserve().unwrap();
        let peer = io.book.reserve().unwrap();
        let mut tickets = vec![first, peer];
        let error = pcm_worker::playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut pcm_worker::PacketOrder::default(),
            &session,
            0,
        )
        .unwrap_err();
        assert_eq!(io.completes, 1);
        assert!(io.book.check(first).is_err());
        assert!(io.book.check(peer).is_ok());
        assert_eq!(io.book.pending(), 1, "peer still requires native drain");
        error
    };
    assert!(matches!(&error, BackendError::ResourceUnavailable(text)
        if text.contains("GetOverlappedResult(VAD PCM)") && text.contains("8007051A")));
    assert_eq!(
        control.current.stream_generation(),
        queried.stream_generation()
    );
    assert!(control.current.gate_open());
    assert_eq!(control.current.gate_epoch(), 6);
    let queries = control.query_calls;
    assert_eq!(
        transport::after_io_error(&mut control, queried, &session, error.clone()),
        Err(error)
    );
    assert_eq!(control.query_calls, queries + 1);
    assert!(control.gates.is_empty());
    assert!(
        session.admitted(),
        "fault diagnostics cannot revoke admission"
    );
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);
    let telemetry = session.telemetry_snapshot();
    assert_eq!(telemetry.recovery_events, 0);
    assert_eq!(telemetry.epoch_resets, 0);
    assert!(!session.drained.load(Ordering::Acquire));
    let mut samples = [0.; 2];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        0
    );
}
