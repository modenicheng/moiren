use super::*;

#[path = "sequence_tests.rs"]
mod sequence_tests;

#[path = "input_capture_tests.rs"]
mod input_capture_tests;
#[path = "reprime_tests.rs"]
mod reprime_tests;
use crate::vad::{SampleFormat, pcm_slots::SlotBook};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::{Engine, EngineConfig},
};
use std::{
    cell::{Cell, RefCell},
    sync::mpsc,
};

struct FakeClock {
    now: Cell<Instant>,
    sleeps: RefCell<Vec<Duration>>,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            now: Cell::new(Instant::now()),
            sleeps: RefCell::new(vec![]),
        }
    }
}

impl FakeClock {
    fn now(&self) -> Instant {
        self.now.get()
    }

    fn advance(&self, elapsed: Duration) {
        self.now.set(self.now() + elapsed);
    }

    fn sleep(&self, elapsed: Duration) {
        self.sleeps.borrow_mut().push(elapsed);
        self.advance(elapsed);
    }
}

thread_local! {
    static BEFORE_PLAYBACK_PUBLISH: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static AFTER_PLAYBACK_PUBLISH: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static BEFORE_DECODE_CALL: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn before_playback_publish() {
    BEFORE_PLAYBACK_PUBLISH.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

pub(super) fn after_playback_publish() {
    AFTER_PLAYBACK_PUBLISH.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

pub(super) fn before_decode_call() {
    BEFORE_DECODE_CALL.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[test]
fn publication_telemetry_reader_exit_never_claims_carrier_publication() {
    let state = state();
    bind_publication(&state);
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    BEFORE_DECODE_CALL.with(|hook| *hook.borrow_mut() = Some(Box::new(move || drop(reader))));
    io.slots[tickets[0].slot].ready = true;
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
    let snapshot = state.telemetry.snapshot().playback_publication.unwrap();
    assert_eq!(snapshot.successful_decode_calls, 0);
    assert_eq!(snapshot.carrier_full_packets, 1);
    assert_eq!(snapshot.carrier_publication_effect_known, None);
}

struct Slot {
    ready: bool,
    stamp: PcmPacketStamp,
    packet: Vec<u8>,
    capture: bool,
    wrong_shape: bool,
    dropped: bool,
}
struct FakeIo {
    book: SlotBook,
    slots: [Slot; crate::vad::REQUEST_SLOTS],
    reads: u64,
    written: Vec<Vec<u8>>,
    collected: Vec<u64>,
    waits: Vec<(PcmTicket, Duration)>,
    signal_on_wait: bool,
    /// Stereo frames per fabricated playback READ packet; 1 keeps the
    /// historical single-frame behavior used by the pump-only tests above.
    render_frames: usize,
    /// Constant sample value for those packets; None keeps sequence numbers.
    render_value: Option<f32>,
    /// Next `read` calls rejected with a transient kernel quota error.
    defer_reads: u32,
    /// Next `complete` calls finishing a ticket with a kernel quota error.
    quota_completions: u32,
    /// Next `write` calls rejected with a transient kernel quota error.
    quota_writes: u32,
}
impl Default for FakeIo {
    fn default() -> Self {
        Self {
            book: SlotBook::new().unwrap(),
            slots: std::array::from_fn(|_| Slot {
                ready: false,
                stamp: PcmPacketStamp {
                    sequence: 0,
                    qpc: 1,
                },
                packet: vec![0; 8],
                capture: false,
                wrong_shape: false,
                dropped: false,
            }),
            reads: 0,
            written: vec![],
            collected: vec![],
            waits: vec![],
            signal_on_wait: false,
            render_frames: 1,
            render_value: None,
            defer_reads: 0,
            quota_completions: 0,
            quota_writes: 0,
        }
    }
}
impl FakeIo {
    fn drain(&mut self, tickets: &mut Vec<PcmTicket>) {
        for ticket in tickets.drain(..) {
            if self.book.check(ticket).is_ok() {
                self.book.finish(ticket).unwrap();
            }
        }
    }
}
impl PcmIo for FakeIo {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), ClientError> {
        self.book.check(ticket).unwrap();
        self.waits.push((ticket, timeout));
        if self.signal_on_wait {
            self.slots[ticket.slot].ready = true;
        }
        Ok(())
    }
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        if self.quota_completions > 0 {
            // Mirror the real owner: the request reached terminal status and
            // finished its slot; the quota rejection is for the pump to defer.
            self.quota_completions -= 1;
            self.book.finish(ticket).unwrap();
            return Err(ClientError::KernelPcmQuotaFull);
        }
        let slot = &mut self.slots[ticket.slot];
        if !slot.ready {
            return Ok(None);
        }
        slot.ready = false;
        self.book.finish(ticket).unwrap();
        self.collected.push(slot.stamp.sequence);
        Ok(Some(if slot.capture && slot.dropped {
            PcmCompletion::CaptureDropped { stamp: slot.stamp }
        } else if slot.capture {
            PcmCompletion::Capture {
                stamp: slot.stamp,
                bytes: slot.packet.len() + usize::from(slot.wrong_shape),
            }
        } else {
            PcmCompletion::Render {
                stamp: slot.stamp,
                pcm: &slot.packet,
            }
        }))
    }
    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        if self.defer_reads > 0 {
            self.defer_reads -= 1;
            return Err(ClientError::KernelPcmQuotaFull);
        }
        let ticket = self.book.reserve().unwrap();
        self.reads += 1;
        let value = self.render_value.unwrap_or(self.reads as f32);
        self.slots[ticket.slot] = Slot {
            ready: false,
            stamp: PcmPacketStamp {
                sequence: self.reads,
                qpc: self.reads,
            },
            packet: vec![value; self.render_frames * 2]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
            capture: false,
            wrong_shape: false,
            dropped: false,
        };
        Ok(ticket)
    }
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, ClientError> {
        if self.quota_writes > 0 {
            self.quota_writes -= 1;
            return Err(ClientError::KernelPcmQuotaFull);
        }
        let ticket = self.book.reserve().unwrap();
        self.written.push(packet.to_vec());
        self.slots[ticket.slot] = Slot {
            ready: false,
            stamp: PcmPacketStamp {
                sequence: self.written.len() as u64,
                qpc: self.written.len() as u64,
            },
            packet: packet.to_vec(),
            capture: true,
            wrong_shape: false,
            dropped: false,
        };
        Ok(ticket)
    }
}
fn state() -> SessionState {
    SessionState::new(Arc::new(Admission::new()))
}

#[test]
fn publication_telemetry_counts_only_accepted_decode_calls_not_stale_placeholders() {
    let state = state();
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    for ticket in tickets.iter().take(2) {
        io.slots[ticket.slot].ready = true;
        io.slots[ticket.slot].stamp = PcmPacketStamp {
            sequence: 1,
            qpc: 1,
        };
    }
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
        PumpState::Current
    );
    let snapshot = serde_json::to_value(state.telemetry.snapshot()).unwrap();
    assert_eq!(
        snapshot["playback_publication"]["successful_decode_calls"],
        1
    );
    assert_eq!(snapshot["playback_publication"]["decode_ok_frames"], 1);
    assert_eq!(snapshot["playback_publication"]["stale_packets"], 1);
    assert_eq!(snapshot["completed_packets"], 2);
    let mut samples = [0.; 4];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(samples, [1., 1., 0., 0.]);
}

fn bind_publication(state: &SessionState) {
    state.active.store(true, Ordering::Release);
    let ticket = state.admission.set(Some(1));
    state.admitted_ticket.store(ticket, Ordering::Release);
    state.admitted_revision.store(1, Ordering::Release);
    state
        .telemetry
        .publication
        .bind(super::super::telemetry::PublicationIdentity {
            native_stamp: [1, 2, 3, 4, 5],
            reset_epoch: 0,
            admission_ticket: ticket,
            active_revision: 1,
            format: SampleFormat::Float32 as u32,
            channels: 2,
            packet_frames: 1,
        });
}

#[test]
fn publication_telemetry_real_pump_keeps_full_and_decode_errors_out_of_success_count() {
    for malformed in [false, true] {
        let state = state();
        bind_publication(&state);
        let mut io = FakeIo::default();
        let mut tickets = pending_reads(&mut io);
        let (mut writer, _reader) = audio_bridge(2, 2, 4096).unwrap();
        if !malformed {
            assert!(writer.write_interleaved_exact(&[1.; 4]).unwrap());
        }
        let mut bridge =
            PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer)
                .unwrap();
        let first = &mut io.slots[tickets[0].slot];
        first.ready = true;
        if malformed {
            first.packet.pop();
        }
        super::super::telemetry::publication::set_test_ticks(&[Some(100), Some(110)]);
        let result = playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            0,
        );
        if malformed {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), PumpState::Reset);
        }
        let snapshot = state.telemetry.snapshot();
        let s = snapshot.playback_publication.unwrap();
        assert_eq!(s.successful_decode_calls, 0);
        assert_eq!(s.valid_call_intervals, 0);
        assert_eq!(s.carrier_full_packets, u64::from(!malformed));
        assert_eq!(s.decode_error_packets, u64::from(malformed));
        assert_eq!(snapshot.completed_packets, 1);
        assert_eq!(snapshot.carrier_dropped_packets, u64::from(!malformed));
    }
}

#[test]
fn publication_telemetry_real_pump_delayed_publication_and_post_write_revocation_are_distinct() {
    let state = Arc::new(state());
    bind_publication(&state);
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    for (index, ticks) in [(0, [Some(100), Some(110)]), (1, [Some(120), Some(10_000)])] {
        io.slots[tickets[0].slot].ready = true;
        super::super::telemetry::publication::set_test_ticks(&ticks);
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap();
        assert_eq!(state.telemetry.snapshot().completed_packets, index + 1);
    }
    let s = state.telemetry.snapshot().playback_publication.unwrap();
    let frequency = s.qpc_frequency_hz.unwrap();
    let expected = (9_900u128 * 10_000_000).div_ceil(u128::from(frequency));
    assert_eq!(s.max_call_interval_100ns, Some(expected as u64));
    assert_eq!(s.valid_call_intervals, 1);
    let revoke = state.clone();
    AFTER_PLAYBACK_PUBLISH
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || revoke.revoke())));
    io.slots[tickets[0].slot].ready = true;
    super::super::telemetry::publication::set_test_ticks(&[Some(11_000), Some(11_010)]);
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Current
    );
    let s = state.telemetry.snapshot().playback_publication.unwrap();
    assert_eq!(
        s.successful_decode_calls, 3,
        "do not erase a write that really happened"
    );
    assert_eq!(s.ambiguous_packets, 1);
    assert_eq!(s.valid_call_intervals, 1);
    assert!(s.invalidated);
    let mut samples = [0.; 6];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        3
    );
    assert_eq!(samples, [1., 1., 2., 2., 3., 3.]);
}

#[test]
fn publication_telemetry_real_pump_epoch_race_does_not_change_audio_reset_decision() {
    let state = Arc::new(state());
    bind_publication(&state);
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    let reset = state.clone();
    AFTER_PLAYBACK_PUBLISH.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            reset.reset_epoch.store(1, Ordering::Release);
        }))
    });
    io.slots[tickets[0].slot].ready = true;
    super::super::telemetry::publication::set_test_ticks(&[Some(100), Some(110)]);
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Reset
    );
    let s = state.telemetry.snapshot().playback_publication.unwrap();
    assert_eq!(s.successful_decode_calls, 1);
    assert_eq!(s.ambiguous_packets, 1);
    assert_eq!(s.valid_call_intervals, 0);
    let mut samples = [0.; 2];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(samples, [1., 1.]);
}

#[test]
fn publication_telemetry_actual_pump_adds_no_allocation_or_deallocation() {
    let state = state();
    bind_publication(&state);
    let mut io = FakeIo::default();
    // Avoid refill-side FakeIo Vec allocation; the real owner uses prepared
    // slots. Keep one ready front packet and three pending slots after it.
    io.collected.reserve(crate::vad::REQUEST_SLOTS);
    let mut tickets = pending_reads(&mut io);
    io.defer_reads = 1;
    io.slots[tickets[0].slot].ready = true;
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    super::super::telemetry::publication::set_test_ticks(&[Some(100), Some(110)]);
    let result = crate::test_allocation::assert_allocates_nothing(|| {
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0)
    });
    assert_eq!(result.unwrap(), PumpState::Current);
    assert_eq!(
        state
            .telemetry
            .snapshot()
            .playback_publication
            .unwrap()
            .successful_decode_calls,
        1
    );
    let mut samples = [0.; 2];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(samples, [1., 1.]);
}

#[test]
fn pcm_cancel_keeps_terminal_cancellation_typed_until_transport_checks_lifecycle() {
    assert!(matches!(
        pcm_error(ClientError::Cancelled),
        BackendError::Cancelled
    ));
    let code = windows::Win32::Foundation::ERROR_DEVICE_NOT_CONNECTED.to_hresult();
    let error = pcm_error(ClientError::windows(
        "GetOverlappedResult(VAD PCM)",
        windows::core::Error::from_hresult(code),
    ));
    assert!(matches!(error, BackendError::ResourceUnavailable(_)));
    assert!(error.to_string().contains("GetOverlappedResult(VAD PCM)"));
}
fn pending_reads(io: &mut FakeIo) -> Vec<PcmTicket> {
    (0..crate::vad::REQUEST_SLOTS)
        .map(|_| io.read().unwrap())
        .collect()
}

fn playback_engine(
    reader: AudioReader<f32>,
    state: &SessionState,
) -> (Engine<f32>, AudioReader<f32>) {
    let source = crate::virtual_owner::VirtualPlaybackSource::new(reader, 2, 1, 48_000, 4096)
        .unwrap()
        .with_reset_epoch(state.reset_epoch.clone())
        .with_epoch_handshake(
            state.producer_quiesced_epoch.clone(),
            state.consumer_quiesced_epoch.clone(),
        );
    let (output, reader) = audio_bridge(2, 4, 4096).unwrap();
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(input).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source).unwrap();
    bindings.bind_sink(sink, output).unwrap();
    let engine = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 1,
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
    .engine;
    (engine, reader)
}

#[test]
fn playback_collects_all_four_in_submission_order_and_waits_for_oldest() {
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    let state = state();
    for ticket in tickets.iter().skip(1) {
        io.slots[ticket.slot].ready = true;
    }
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Current
    );
    assert!(io.collected.is_empty());
    io.slots[tickets[0].slot].ready = true;
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap();
    let mut samples = [0.0; 8];
    reader.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [1., 1., 2., 2., 3., 3., 4., 4.]);
    assert_eq!(io.collected, vec![1, 2, 3, 4]);
    assert_eq!(tickets.len(), 4);
}

#[test]
fn playback_reset_waits_for_late_old_publication_before_discard() {
    let state = Arc::new(state());
    let (writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    let (mut engine, mut output) = playback_engine(reader, &state);
    let (paused_tx, paused_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let worker_state = state.clone();
    let worker = thread::spawn(move || {
        let mut io = FakeIo::default();
        let mut tickets = pending_reads(&mut io);
        io.slots[tickets[0].slot].ready = true;
        let mut bridge =
            PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer)
                .unwrap();
        BEFORE_PLAYBACK_PUBLISH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                paused_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }));
        });
        assert_eq!(
            playback(
                &mut io,
                &mut bridge,
                &mut tickets,
                &mut PacketOrder::default(),
                &worker_state,
                0,
            )
            .unwrap(),
            PumpState::Reset
        );
        (io, bridge, tickets)
    });
    paused_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    state.reset_epoch.fetch_add(1, Ordering::AcqRel);
    engine.render(1).unwrap();
    let mut samples = [f32::NAN; 2];
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.; 2]);
    resume_tx.send(()).unwrap();
    let (mut io, bridge, mut tickets) = worker.join().unwrap();
    engine.render(1).unwrap();
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(
        samples, [0.; 2],
        "late old publication must not cross reset"
    );
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 0);

    let mut writer = None;
    let mut observed = 0;
    assert!(!quiesce_playback(&state, &mut observed, || {
        io.drain(&mut tickets);
        writer = Some(bridge.into_carrier());
        assert_eq!(state.producer_quiesced_epoch.load(Ordering::Acquire), 0);
    }));
    assert_eq!(state.producer_quiesced_epoch.load(Ordering::Acquire), 1);
    assert_eq!(
        observed, 0,
        "producer cannot resume before consumer discard"
    );
    engine.render(1).unwrap();
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(
        samples, [0.; 2],
        "acknowledged discard removes late old PCM"
    );
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 1);
    assert!(quiesce_playback(&state, &mut observed, || {}));
    assert_eq!(observed, 1);

    let mut bridge = PcmPlaybackBridge::new(
        Channels::Stereo,
        SampleFormat::Float32,
        1,
        4096,
        writer.unwrap(),
    )
    .unwrap();
    let mut order = PacketOrder::default();
    playback(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut order,
        &state,
        observed,
    )
    .unwrap();
    io.slots[tickets[0].slot].ready = true;
    playback(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut order,
        &state,
        observed,
    )
    .unwrap();
    engine.render(1).unwrap();
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [5.; 2], "fresh PCM flows after the two-way fence");
}

#[test]
fn playback_discards_late_and_duplicate_packet_stamps() {
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    for (ticket, sequence) in tickets.iter().zip([2, 1, 3, 3]) {
        io.slots[ticket.slot].ready = true;
        io.slots[ticket.slot].stamp = PcmPacketStamp {
            sequence,
            qpc: sequence,
        };
    }
    playback(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut PacketOrder::default(),
        &state(),
        0,
    )
    .unwrap();
    let mut samples = [0.0; 8];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        2
    );
    assert_eq!(samples, [1., 1., 3., 3., 0., 0., 0., 0.]);
}

#[test]
fn stalled_playback_requests_consumer_discard_instead_of_replaying_full_queue() {
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, reader) = audio_bridge(2, 1, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    for ticket in &tickets {
        io.slots[ticket.slot].ready = true;
    }
    let state = state();
    let (mut engine, mut output) = playback_engine(reader, &state);
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
    assert_eq!(state.reset_epoch.load(Ordering::Acquire), 1);
    let mut observed = 0;
    let mut writer = None;
    assert!(!quiesce_playback(&state, &mut observed, || {
        io.drain(&mut tickets);
        writer = Some(bridge.into_carrier());
    }));
    engine.render(1).unwrap();
    let mut samples = [f32::NAN; 2];
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(
        samples,
        [0., 0.],
        "the real prepared source drops its stale queue"
    );
    assert!(quiesce_playback(&state, &mut observed, || {}));
    let mut bridge = PcmPlaybackBridge::new(
        Channels::Stereo,
        SampleFormat::Float32,
        1,
        4096,
        writer.unwrap(),
    )
    .unwrap();
    let mut order = PacketOrder::default();
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 1).unwrap();
    io.slots[tickets[0].slot].ready = true;
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 1).unwrap();
    engine.render(1).unwrap();
    output.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [5., 5.]);
}

#[test]
fn microphone_clock_reset_stops_old_slots_before_encoding_fresh_carrier() {
    let mut io = FakeIo::default();
    let (mut writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    writer.write_interleaved(&[1.0; 8]).unwrap();
    let mut bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap();
    let mut tickets = vec![];
    let mut pump = MicrophonePump::new(1);
    let clock = FakeClock::default();
    let state = state();
    state.active.store(true, Ordering::Release);
    let ticket = state.admission.set(Some(9));
    state.admitted_ticket.store(ticket, Ordering::Release);
    state.admitted_revision.store(9, Ordering::Release);
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(io.written.len(), 1);
    writer.write_interleaved(&[2.0; 8]).unwrap();
    state.reset_epoch.fetch_add(1, Ordering::AcqRel); // The virtual clock only touches this atomic.
    assert_eq!(
        microphone(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut pump,
            &state,
            0,
            || clock.now()
        )
        .unwrap(),
        PumpState::Reset
    );
    assert_eq!(io.written.len(), 1);
    io.drain(&mut tickets);
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = Some(bridge);
    let mut reader = None;
    super::super::teardown_microphone(
        &mut super::super::RetiredPcm::new(),
        &mut owner,
        &mut bridge,
        &mut reader,
        &mut tickets,
    );
    let mut observed = 0;
    assert!(!quiesce_microphone(
        &state,
        &mut observed,
        reader.as_mut().unwrap()
    ));
    state.producer_quiesced_epoch.store(1, Ordering::Release);
    assert!(quiesce_microphone(
        &state,
        &mut observed,
        reader.as_mut().unwrap()
    ));
    writer.write_interleaved(&[3.0; 8]).unwrap();
    let mut bridge = PcmMicrophoneBridge::new(
        Channels::Stereo,
        SampleFormat::Float32,
        1,
        4096,
        reader.unwrap(),
    )
    .unwrap();
    assert_eq!(observed, 1);
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut MicrophonePump::new(1),
        &state,
        observed,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(io.written.len(), 2);
    for packet in &io.written[1..] {
        assert_eq!(
            packet,
            &[3.0_f32; 2]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        );
    }
    assert!(
        state.admitted(),
        "clock resets must not change native admission"
    );
}

#[test]
fn microphone_reset_waits_for_late_graph_write_before_discard() {
    let clock = FakeClock::default();
    let state = Arc::new(state());
    let (mut writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    writer.write_interleaved(&[7.; 2]).unwrap();
    let mut bridge = Some(
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap(),
    );
    let (paused_tx, paused_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let graph_state = state.clone();
    let producer = thread::spawn(move || {
        assert_eq!(graph_state.reset_epoch.load(Ordering::Acquire), 0);
        paused_tx.send(()).unwrap();
        resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // The callback already passed its last epoch check before it paused.
        writer.write_interleaved(&[9.; 2]).unwrap();
        graph_state
            .producer_quiesced_epoch
            .store(1, Ordering::Release);
        writer
    });
    paused_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    state.reset_epoch.fetch_add(1, Ordering::AcqRel);
    let mut owner = None;
    let mut reader = None;
    let mut tickets = vec![];
    let mut io = FakeIo::default();
    assert_eq!(
        microphone(
            &mut io,
            bridge.as_mut().unwrap(),
            &mut tickets,
            &mut MicrophonePump::new(1),
            &state,
            1,
            || clock.now(),
        )
        .unwrap(),
        PumpState::Reset,
        "even a new-epoch consumer cannot encode before graph quiescence"
    );
    assert!(io.written.is_empty());
    super::super::teardown_microphone(
        &mut super::super::RetiredPcm::new(),
        &mut owner,
        &mut bridge,
        &mut reader,
        &mut tickets,
    );
    let mut reader = reader.unwrap();
    let mut observed = 0;
    assert!(!quiesce_microphone(&state, &mut observed, &mut reader));
    assert_eq!(observed, 0);
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 0);
    resume_tx.send(()).unwrap();
    let mut writer = producer.join().unwrap();
    assert!(quiesce_microphone(&state, &mut observed, &mut reader));
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 1);
    let mut samples = [f32::NAN; 4];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        0
    );
    assert_eq!(
        samples, [0.; 4],
        "both early and late old PCM were discarded"
    );

    writer.write_interleaved(&[3.; 8]).unwrap();
    let mut bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap();
    for _ in 0..2 {
        assert_eq!(
            microphone(
                &mut io,
                &mut bridge,
                &mut tickets,
                &mut MicrophonePump::new(1),
                &state,
                0,
                || clock.now(),
            )
            .unwrap(),
            PumpState::Reset,
            "a duplicate old pump cannot consume fresh carrier data"
        );
    }
    assert!(io.written.is_empty());
    assert_eq!(
        microphone(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut MicrophonePump::new(1),
            &state,
            observed,
            || clock.now(),
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(io.written.len(), 1);
    let expected = [3.0_f32; 2]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    assert!(io.written.iter().all(|packet| packet == &expected));
}

#[test]
fn repeated_playback_reset_does_not_accept_an_older_consumer_ack() {
    let state = state();
    let mut observed = 0;
    state.reset_epoch.store(1, Ordering::Release);
    assert!(!quiesce_playback(&state, &mut observed, || {}));
    state.consumer_quiesced_epoch.store(1, Ordering::Release);
    state.reset_epoch.store(2, Ordering::Release);
    assert!(!quiesce_playback(&state, &mut observed, || {}));
    assert_eq!(observed, 0);
    assert_eq!(state.producer_quiesced_epoch.load(Ordering::Acquire), 2);
    state.consumer_quiesced_epoch.store(2, Ordering::Release);
    assert!(quiesce_playback(&state, &mut observed, || {}));
    assert_eq!(observed, 2);
}

#[test]
fn repeated_microphone_reset_does_not_accept_an_older_producer_ack() {
    let state = state();
    let (mut writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut observed = 0;
    state.reset_epoch.store(2, Ordering::Release);
    state.producer_quiesced_epoch.store(1, Ordering::Release);
    writer.write_interleaved(&[9.; 2]).unwrap();
    assert!(!quiesce_microphone(&state, &mut observed, &mut reader));
    assert_eq!(observed, 0);
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 0);
    state.producer_quiesced_epoch.store(2, Ordering::Release);
    assert!(quiesce_microphone(&state, &mut observed, &mut reader));
    assert_eq!(observed, 2);
    assert_eq!(state.consumer_quiesced_epoch.load(Ordering::Acquire), 2);
    assert_eq!(reader.discard(), 0);
}

#[test]
fn playback_pump_cannot_publish_fresh_pcm_before_consumer_discard() {
    let state = state();
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    io.slots[tickets[0].slot].ready = true;
    state.reset_epoch.store(1, Ordering::Release);
    state.producer_quiesced_epoch.store(1, Ordering::Release);
    assert_eq!(
        playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            1
        )
        .unwrap(),
        PumpState::Reset
    );
    assert!(io.collected.is_empty());
    assert_eq!(reader.discard(), 0);
    state.consumer_quiesced_epoch.store(1, Ordering::Release);
    for _ in 0..2 {
        assert_eq!(
            playback(
                &mut io,
                &mut bridge,
                &mut tickets,
                &mut PacketOrder::default(),
                &state,
                0,
            )
            .unwrap(),
            PumpState::Reset,
            "consumer acknowledgment cannot revive a duplicate old pump"
        );
    }
    assert!(io.collected.is_empty());
    assert_eq!(
        playback(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut PacketOrder::default(),
            &state,
            1
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(reader.discard(), 1);
}

#[test]
fn microphone_completion_order_and_shape_are_checked() {
    let mut io = FakeIo::default();
    let (_, reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap();
    let mut tickets = vec![];
    let mut pump = MicrophonePump::new(1);
    let clock = FakeClock::default();
    let state = state();
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    for ticket in &tickets {
        io.slots[ticket.slot].ready = true;
    }
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(io.collected, vec![1]);
    assert!(tickets.is_empty());
    clock.advance(pump.half_period);
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    io.slots[tickets[0].slot].ready = true;
    io.slots[tickets[0].slot].wrong_shape = true;
    assert!(
        microphone(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut pump,
            &state,
            0,
            || clock.now()
        )
        .is_err()
    );
}

#[test]
fn microphone_submits_one_packet_and_replenishes_only_after_completion() {
    let state = state();
    let (mut writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    writer
        .write_interleaved(&[1., 1., 2., 2., 3., 3., 4., 4.])
        .unwrap();
    let mut bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap();
    let mut io = FakeIo::default();
    let mut tickets = vec![];
    let mut pump = MicrophonePump::new(1);
    let clock = FakeClock::default();
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(
        tickets.len(),
        1,
        "never timestamp several future native periods as now"
    );
    assert_eq!(io.written.len(), 1);
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(io.written.len(), 1, "pending packet must not encode ahead");
    io.slots[tickets[0].slot].ready = true;
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert!(
        tickets.is_empty(),
        "completion must defer the next write instead of filling a whole-period deadline cliff"
    );
    assert_eq!(
        io.written.len(),
        1,
        "do not encode ahead during the phase wait"
    );
    clock.advance(pump.half_period - Duration::from_nanos(1));
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert!(
        tickets.is_empty(),
        "wait through the last nanosecond before the phase"
    );
    clock.advance(Duration::from_nanos(1));
    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        || clock.now(),
    )
    .unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(io.written.len(), 2);
    for (packet, value) in io.written.iter().zip([1.0_f32, 2.0]) {
        assert_eq!(packet, &value.to_le_bytes().repeat(2));
    }
}

#[test]
fn microphone_pump_records_native_submission_completion_and_silence_metrics() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();

    let initial = f.state.telemetry.snapshot();
    assert_eq!(initial.packet_frames, 480);
    assert_eq!(initial.submission_attempts, 1);
    assert_eq!(initial.submitted_packets, 1);
    assert_eq!(initial.completed_packets, 0);
    assert_eq!(initial.silence_packets, 1);
    assert_eq!(initial.silence_frames, 480);

    f.collect(false);
    let completed = f.state.telemetry.snapshot();
    assert_eq!(completed.completed_packets, 1);
    assert_eq!(completed.completion_interval_observations, 0);
    assert_eq!(completed.microphone_submission_service.observations, 1);
    assert_eq!(completed.playback_transfer_service.observations, 0);
}

#[test]
fn playback_defers_kernel_quota_reads_without_failing_the_session() {
    let mut io = FakeIo {
        defer_reads: 1,
        ..Default::default()
    };
    let mut tickets = vec![];
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    let state = state();
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Current,
        "a transient kernel quota rejection must defer the fill, not fail"
    );
    assert!(
        tickets.is_empty(),
        "a deferred read must not leave a ticket behind"
    );
    assert_eq!(io.reads, 0);
    // The next pass submits normally once the retired owner's cancelled
    // requests stop occupying the endpoint quota.
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Current
    );
    assert_eq!(tickets.len(), crate::vad::REQUEST_SLOTS);
}

#[test]
fn playback_treats_kernel_quota_completion_as_terminal_ticket_and_refills() {
    let mut io = FakeIo {
        quota_completions: 1,
        ..Default::default()
    };
    let mut tickets = pending_reads(&mut io);
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    let state = state();
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Current,
        "a quota-rejected completion must not poison the session"
    );
    assert_eq!(
        tickets.len(),
        crate::vad::REQUEST_SLOTS,
        "the finished ticket is dropped and the fill resubmits"
    );
    assert!(
        io.collected.is_empty(),
        "a quota completion carries no packet"
    );
    assert_eq!(
        io.reads,
        u64::try_from(crate::vad::REQUEST_SLOTS + 1).unwrap()
    );
}

#[test]
fn microphone_treats_kernel_quota_write_as_a_bounded_drop() {
    let state = state();
    let (mut writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    writer
        .write_interleaved(&[1., 1., 2., 2., 3., 3., 4., 4.])
        .unwrap();
    let mut bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, reader).unwrap();
    let mut io = FakeIo {
        quota_writes: 1,
        ..Default::default()
    };
    let mut tickets = vec![];
    let mut pump = MicrophonePump::new(1);
    let clock = FakeClock::default();
    assert_eq!(
        microphone(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut pump,
            &state,
            0,
            || clock.now(),
        )
        .unwrap(),
        PumpState::Current,
        "a quota-rejected WRITE is bounded backpressure, not a session fault"
    );
    assert!(
        tickets.is_empty(),
        "the rejected packet leaves no ticket; its frames are dropped"
    );
    assert!(io.written.is_empty());
    // The phase stays due, so the very next pass encodes the next packet —
    // the cadence continues and the loss stays bounded to one packet.
    assert_eq!(
        microphone(
            &mut io,
            &mut bridge,
            &mut tickets,
            &mut pump,
            &state,
            0,
            || clock.now(),
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(tickets.len(), 1);
    assert_eq!(io.written.len(), 1);
    assert_eq!(io.written[0], 2.0_f32.to_le_bytes().repeat(2));
}

struct MicrophoneFixture {
    io: FakeIo,
    writer: AudioWriter<f32>,
    bridge: PcmMicrophoneBridge,
    tickets: Vec<PcmTicket>,
    pump: MicrophonePump,
    state: SessionState,
    clock: FakeClock,
}

impl MicrophoneFixture {
    fn new(frames: u32) -> Self {
        let (writer, reader) = audio_bridge(2, frames as usize * 2, 1 << 20).unwrap();
        Self {
            io: FakeIo::default(),
            writer,
            bridge: PcmMicrophoneBridge::new(
                Channels::Stereo,
                SampleFormat::Float32,
                frames as usize,
                1 << 20,
                reader,
            )
            .unwrap(),
            tickets: vec![],
            pump: MicrophonePump::new(frames),
            state: state(),
            clock: FakeClock::default(),
        }
    }

    fn cycle(&mut self) -> Result<PumpState, BackendError> {
        microphone(
            &mut self.io,
            &mut self.bridge,
            &mut self.tickets,
            &mut self.pump,
            &self.state,
            0,
            || self.clock.now(),
        )
    }

    fn collect(&mut self, dropped: bool) {
        let ticket = self.tickets[0];
        self.io.slots[ticket.slot].ready = true;
        self.io.slots[ticket.slot].dropped = dropped;
        self.cycle().unwrap();
        assert!(self.tickets.is_empty());
    }

    fn wait(&mut self) -> Result<(), BackendError> {
        wait_for_microphone(
            &mut self.io,
            &self.tickets,
            &mut self.pump,
            &self.state,
            0,
            self.clock.now(),
            |duration| self.clock.sleep(duration),
        )
    }
}

#[test]
fn microphone_observation_gap_soft_resets_phase_and_keeps_carrier_content() {
    let mut f = MicrophoneFixture::new(480);
    f.state.active.store(true, Ordering::Release);
    let admitted = f.state.admission.set(Some(9));
    f.state.admitted_ticket.store(admitted, Ordering::Release);
    f.state.admitted_revision.store(9, Ordering::Release);
    f.cycle().unwrap();
    let pending = f.tickets[0];
    f.io.slots[pending.slot].ready = true;
    // A shallow, lossless publication followed by a stopped producer: the old
    // design fenced and discarded for this; the graded design keeps the
    // in-order content and only drops the stale phase anchor.
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(100));
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.collected.len(), 1, "collection proceeds after the gap");
    assert!(f.tickets.is_empty());
    assert_eq!(
        f.io.written.len(),
        1,
        "no submission before the re-armed half period"
    );
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert!(
        f.state.admitted(),
        "a timeline gap must not revoke admission"
    );
    assert!(f.state.failure.lock().unwrap().is_none());
    // Once the fresh half period is due, the queued graph content flows
    // through untouched.
    f.clock.advance(Duration::from_millis(5));
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written[1], 1.0_f32.to_le_bytes().repeat(960));
    f.collect(false);
    f.writer.write_interleaved_exact(&vec![5.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(10));
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written[2], 5.0_f32.to_le_bytes().repeat(960));
}

#[test]
fn microphone_observation_gap_during_phase_encodes_with_a_fresh_phase() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(100));
    // Soft: the phase anchor is stale, the content is not. The encode goes
    // out immediately with a fresh anchor; no epoch moves.
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.io.written[1], 1.0_f32.to_le_bytes().repeat(960));
    assert!(f.pump.last_phase.is_none());
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
}

#[test]
fn microphone_observation_gap_threshold_is_one_period_plus_io_wait() {
    for (frames, bound) in [
        (48, Duration::from_millis(11)),
        (49, Duration::from_nanos(11_020_834)),
        (480, Duration::from_millis(20)),
        (4800, Duration::from_millis(110)),
    ] {
        let state = state();
        let mut pump = MicrophonePump::new(frames);
        let start = Instant::now();
        assert!(pump.observe(&state, 0, start));
        pump.next_submit = Some(start + Duration::from_secs(3600));
        // At exactly the bound the phase anchor survives...
        assert!(pump.observe(&state, 0, start + bound));
        assert!(
            pump.next_submit.is_some(),
            "gap == bound must not clear the phase"
        );
        assert_eq!(state.reset_epoch.load(Ordering::Acquire), 0);
        // ...and the first step beyond it (1 us: the smallest increment
        // reliably representable above a tick-aligned Instant), measured
        // between consecutive observations, softly clears the anchor: no
        // fence, no epoch bump.
        pump.next_submit = Some(start + Duration::from_secs(3600));
        assert!(pump.observe(&state, 0, start + bound + bound + Duration::from_micros(1)));
        assert!(
            pump.next_submit.is_none(),
            "gap > bound softly clears the phase"
        );
        assert!(pump.last_phase.is_none());
        assert_eq!(state.reset_epoch.load(Ordering::Acquire), 0);
    }
}

#[test]
fn microphone_observation_gap_immediately_before_completion_collects_and_rearms() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    let pending = f.tickets[0];
    f.io.slots[pending.slot].ready = true;
    let start = f.clock.now();
    let mut observations = 0;
    assert_eq!(
        microphone(
            &mut f.io,
            &mut f.bridge,
            &mut f.tickets,
            &mut f.pump,
            &f.state,
            0,
            || {
                observations += 1;
                if observations == 1 {
                    start
                } else {
                    start + Duration::from_millis(100)
                }
            },
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(f.io.collected.len(), 1, "the ready completion is collected");
    assert!(f.tickets.is_empty());
    assert!(f.pump.next_submit.is_some(), "a fresh half period is armed");
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert_eq!(f.io.book.pending(), 0);
}

#[test]
fn microphone_observation_gap_inside_completion_arms_a_fresh_half_period() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.io.slots[f.tickets[0].slot].ready = true;
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    let start = f.clock.now();
    let mut observations = 0;
    assert_eq!(
        microphone(
            &mut f.io,
            &mut f.bridge,
            &mut f.tickets,
            &mut f.pump,
            &f.state,
            0,
            || {
                observations += 1;
                if observations <= 2 {
                    start
                } else {
                    start + Duration::from_millis(100)
                }
            },
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(f.io.collected.len(), 1);
    // The stale phase cannot re-arm: the anchor is a fresh half period from
    // this collection, not period-stepped from before the gap.
    assert_eq!(
        f.pump.next_submit,
        Some(start + Duration::from_millis(100) + f.pump.half_period)
    );
    assert!(f.pump.last_phase.is_none());
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    f.io.drain(&mut f.tickets);
    assert_eq!(f.io.book.pending(), 0);
}

#[test]
fn microphone_observation_gap_between_encode_and_submit_does_not_fence_the_packet() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(5));
    let phase = f.clock.now();
    let mut observations = 0;
    assert_eq!(
        microphone(
            &mut f.io,
            &mut f.bridge,
            &mut f.tickets,
            &mut f.pump,
            &f.state,
            0,
            || {
                observations += 1;
                if observations <= 2 {
                    phase
                } else {
                    phase + Duration::from_millis(100)
                }
            },
        )
        .unwrap(),
        PumpState::Current
    );
    // The encode itself was on time; a gap observed between encode and submit
    // clears the stale anchor softly and the packet still goes out.
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.io.written[1], 1.0_f32.to_le_bytes().repeat(960));
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert!(
        f.pump.last_phase.is_none(),
        "a submission one period past its phase does not keep the anchor"
    );
}

#[test]
fn microphone_reprime_after_a_hard_reset_waits_for_two_packets_or_the_deadline() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.io.drain(&mut f.tickets);
    f.pump.arm_reprime(f.clock.now()).unwrap();
    // One packet of margin is not enough: the first fresh submission waits.
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written.len(), 1, "re-prime holds below two packets");
    f.wait().unwrap();
    assert_eq!(
        f.io.written.len(),
        1,
        "the bounded wait yields without submitting"
    );
    // The bounded deadline (2.5 native periods) expires the wait: the packet
    // goes out with an honest silence tail, never a stalled worker.
    f.clock.advance(Duration::from_millis(26));
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);

    // With two packets queued the submission is immediate.
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.io.drain(&mut f.tickets);
    f.pump.arm_reprime(f.clock.now()).unwrap();
    f.writer.write_interleaved_exact(&vec![7.; 1920]).unwrap();
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.io.written[1], 7.0_f32.to_le_bytes().repeat(960));
}

#[test]
fn microphone_first_observation_after_idle_has_no_elapsed_history() {
    let mut f = MicrophoneFixture::new(480);
    f.clock.advance(Duration::from_secs(60));
    assert_eq!(f.cycle().unwrap(), PumpState::Current);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert_eq!(f.tickets.len(), 1);
    assert_eq!(f.io.written.len(), 1);
}

#[test]
fn microphone_observation_gap_is_not_a_no_transfer_timeout() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    let pending = f.tickets[0];
    for _ in 0..1000 {
        f.clock.advance(POLL_INTERVAL);
        assert_eq!(f.cycle().unwrap(), PumpState::Current);
    }
    assert_eq!(f.tickets, vec![pending]);
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert!(f.state.failure.lock().unwrap().is_none());
}

#[test]
fn microphone_phase_reads_carrier_only_when_due() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    let deadline = f.pump.next_submit.unwrap();
    f.clock.advance(Duration::from_millis(4));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);
    f.writer.write_interleaved_exact(&vec![7.; 960]).unwrap();
    assert_eq!(
        f.pump.next_submit,
        Some(deadline),
        "polling never rearms the phase"
    );
    f.clock.advance(Duration::from_millis(1));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.io.written[1], 7.0_f32.to_le_bytes().repeat(960));
    assert_eq!(f.tickets.len(), 1);
}

#[test]
fn microphone_delayed_collection_does_not_shift_the_next_native_phase() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    // Native consumes at 10 ms; the first collected completion establishes
    // a halfway replenishment phase at 15 ms.
    f.clock.advance(Duration::from_millis(10));
    f.collect(false);
    f.clock.advance(Duration::from_millis(5));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 2);

    // The second request was consumed at 20 ms, but a scheduling delay makes
    // that completion observable at 23 ms. This is not a new native phase.
    f.clock.advance(Duration::from_millis(8));
    f.collect(false);
    f.writer.write_interleaved_exact(&vec![3.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(2));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 3, "preserve the 25 ms phase");
    assert_eq!(f.io.written[2], 3.0_f32.to_le_bytes().repeat(960));
    assert_eq!(f.tickets.len(), 1);
}

#[test]
fn microphone_successful_collection_at_or_past_phase_submits_one_fresh_packet() {
    for collected_ms in [25, 26, 31] {
        let mut f = MicrophoneFixture::new(480);
        f.cycle().unwrap();
        f.clock.advance(Duration::from_millis(10));
        f.collect(false);
        f.clock.advance(Duration::from_millis(5));
        f.cycle().unwrap();
        f.writer.write_interleaved_exact(&vec![4.; 960]).unwrap();
        f.clock.advance(Duration::from_millis(collected_ms - 15));
        let ticket = f.tickets[0];
        f.io.slots[ticket.slot].ready = true;
        f.cycle().unwrap();
        assert_eq!(f.io.written.len(), 3, "collected at {collected_ms} ms");
        assert_eq!(f.io.written[2], 4.0_f32.to_le_bytes().repeat(960));
        f.cycle().unwrap();
        assert_eq!(f.io.written.len(), 3, "no burst catch-up");
        assert_eq!(f.tickets.len(), 1);
    }
}

#[test]
fn microphone_reset_and_drop_clear_an_established_phase() {
    for reset in [false, true] {
        let mut f = MicrophoneFixture::new(480);
        f.cycle().unwrap();
        f.clock.advance(Duration::from_millis(10));
        f.collect(false);
        f.clock.advance(Duration::from_millis(5));
        f.cycle().unwrap();
        assert!(f.pump.last_phase.is_some());
        f.clock.advance(Duration::from_millis(8));
        if reset {
            f.state.reset_epoch.fetch_add(1, Ordering::AcqRel);
            assert_eq!(f.cycle().unwrap(), PumpState::Reset);
            assert!(f.pump.last_phase.is_none());
            assert!(f.pump.next_submit.is_none());
        } else {
            f.collect(true);
            assert_eq!(
                f.pump.next_submit,
                Some(f.clock.now() + Duration::from_millis(5)),
                "an expired request cannot anchor the next native phase"
            );
        }
    }
}

#[test]
fn microphone_dropped_completion_rearms_from_collection_without_catch_up() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    for _ in 0..2000 {
        f.clock.advance(POLL_INTERVAL);
        assert_eq!(f.cycle().unwrap(), PumpState::Current);
    }
    let collected = f.clock.now();
    f.collect(true);
    let dropped = f.state.telemetry.snapshot();
    assert_eq!(dropped.driver_dropped_packets, 1);
    assert_eq!(dropped.microphone_submission_service.observations, 1);
    assert_eq!(dropped.playback_transfer_service.observations, 0);
    assert_eq!(
        f.pump.next_submit,
        Some(collected + Duration::from_millis(5))
    );
    assert_eq!(f.io.written.len(), 1);
    f.clock.advance(Duration::from_millis(15));
    f.cycle().unwrap();
    assert_eq!(
        f.io.written.len(),
        2,
        "a late phase inside the observation bound submits at most one packet"
    );
    f.cycle().unwrap();
    assert_eq!(
        f.io.written.len(),
        2,
        "pending packet prevents catch-up submissions"
    );
    f.collect(false);
    assert_eq!(
        f.pump.next_submit,
        Some(f.clock.now() + Duration::from_millis(5))
    );
    assert_eq!(
        f.state
            .telemetry
            .snapshot()
            .microphone_submission_service
            .observations,
        2
    );
}

#[test]
fn microphone_phase_wait_yields_in_one_ms_slices_and_exact_remaining_tail() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    for _ in 0..4 {
        f.wait().unwrap();
        f.cycle().unwrap();
        assert_eq!(f.io.written.len(), 1);
    }
    f.clock.advance(Duration::from_micros(750));
    f.wait().unwrap();
    assert_eq!(
        &*f.clock.sleeps.borrow(),
        &[
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_micros(250),
        ]
    );
    assert!(
        f.io.waits.is_empty(),
        "no native event exists while no ticket is pending"
    );
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 2);
}

#[test]
fn microphone_pending_wait_is_bounded_and_timeout_does_not_resubmit() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    let ticket = f.tickets[0];
    f.wait().unwrap();
    f.cycle().unwrap();
    assert_eq!(f.io.waits, vec![(ticket, POLL_INTERVAL)]);
    assert!(f.clock.sleeps.borrow().is_empty());
    assert_eq!(f.tickets, vec![ticket]);
    assert_eq!(f.io.written.len(), 1);
    f.pump.next_submit = Some(f.clock.now() + Duration::from_micros(300));
    f.wait().unwrap();
    assert_eq!(
        f.io.waits.last(),
        Some(&(ticket, Duration::from_micros(300)))
    );
}

#[test]
fn microphone_due_phase_never_sleeps_or_waits_an_absent_ticket() {
    let mut f = MicrophoneFixture::new(480);
    f.wait().unwrap();
    assert!(f.clock.sleeps.borrow().is_empty());
    f.cycle().unwrap();
    f.collect(false);
    f.clock.advance(Duration::from_millis(5));
    f.wait().unwrap();
    assert!(f.clock.sleeps.borrow().is_empty());
    assert!(f.io.waits.is_empty());
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 2);
}

#[test]
fn microphone_cancel_and_epoch_reset_clear_wait_and_block_encoding() {
    for cancel in [false, true] {
        let mut f = MicrophoneFixture::new(480);
        f.cycle().unwrap();
        f.collect(false);
        if cancel {
            f.state.cancel.store(true, Ordering::Release);
        } else {
            f.state.reset_epoch.store(1, Ordering::Release);
        }
        f.wait().unwrap();
        f.clock.advance(Duration::from_secs(1));
        assert_eq!(f.cycle().unwrap(), PumpState::Reset);
        assert!(f.pump.next_submit.is_none());
        assert!(f.pump.order.0.is_none());
        assert!(f.clock.sleeps.borrow().is_empty());
        assert!(f.io.waits.is_empty());
        assert_eq!(f.io.written.len(), 1);
    }
}

#[test]
fn microphone_reset_during_phase_sleep_returns_without_submitting() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    wait_for_microphone(
        &mut f.io,
        &f.tickets,
        &mut f.pump,
        &f.state,
        0,
        f.clock.now(),
        |duration| {
            assert_eq!(duration, Duration::from_millis(1));
            f.clock.sleep(duration);
            f.state.reset_epoch.store(1, Ordering::Release);
        },
    )
    .unwrap();
    assert_eq!(f.cycle().unwrap(), PumpState::Reset);
    assert_eq!(f.io.written.len(), 1);
}

#[test]
fn microphone_new_stream_phase_starts_immediately_with_new_shape() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    assert!(f.pump.next_submit.is_some());
    f.pump = MicrophonePump::new(49);
    let (writer, reader) = audio_bridge(2, 98, 131_072).unwrap();
    f.writer = writer;
    f.bridge =
        PcmMicrophoneBridge::new(Channels::Stereo, SampleFormat::Float32, 49, 131_072, reader)
            .unwrap();
    f.io = FakeIo::default();
    f.cycle().unwrap();
    assert_eq!(
        f.tickets.len(),
        1,
        "new stream discards the old pacing deadline"
    );
    assert_eq!(f.pump.half_period, Duration::from_nanos(510_417));
    assert!(f.pump.next_submit.is_none());
}

#[test]
fn activity_wait_targets_oldest_and_timeout_keeps_request_pending() {
    let mut io = FakeIo::default();
    let tickets = pending_reads(&mut io);
    let state = state();
    wait_for_activity(&mut io, &tickets, &state, 0).unwrap();
    assert_eq!(io.waits, [(tickets[0], POLL_INTERVAL)]);
    assert_eq!(
        io.book.pending(),
        4,
        "wait timeout must not cancel or collect"
    );
    assert!(io.complete(tickets[0]).unwrap().is_none());
    io.signal_on_wait = true;
    wait_for_activity(&mut io, &tickets, &state, 0).unwrap();
    assert!(io.complete(tickets[0]).unwrap().is_some());
    let waits = io.waits.len();
    state.reset_epoch.store(1, Ordering::Release);
    wait_for_activity(&mut io, &tickets[1..], &state, 0).unwrap();
    assert_eq!(
        io.waits.len(),
        waits,
        "reset must return to quiescence, not wait"
    );
}

/// Deterministic reproduction of the target-machine combined-soak failure:
/// one skipped Graph256 owner period resets both fenced boundaries, the
/// microphone worker completes its discard handshake and re-arms a fresh
/// pump, and the first post-reset `encode_packet` runs while the Graph has
/// not yet published any fresh frames (the observed `0/480` silent packet).
/// The real prepared owner renders through the real fenced boundaries; only
/// the native PCM IO and both clocks are fakes.
#[cfg(windows)]
mod owner_reset_regression {
    use super::*;
    use crate::virtual_owner::{
        BoundaryProbe, ClockWake, VirtualMicrophoneSink, VirtualPlaybackSource,
        VirtualPreparedTransport, VirtualRealtimeClock, VirtualResourceSession, VirtualSessions,
    };
    use crate::{ActivationGate, StopSignal};
    use moiren_core::graph::NodeId;
    use moiren_engine::compiler::CompiledBindings;
    use moiren_engine::host::PreparedTransport;
    use std::sync::Condvar;

    /// 240-frame engine blocks make the harness rate-exact: one period is
    /// 5 ms, two sub-steps are one native 480-frame packet period, and the
    /// pump's once-per-sub-step phase check quantizes to the true cadence
    /// with zero residual drift. The target runs Graph256; its shape is
    /// revalidated on the machine, while this harness pins the redesign
    /// invariants deterministically.
    const ENGINE_FRAMES: usize = 240;
    const NATIVE_PACKET_FRAMES: usize = 480;
    const CARRIER_FRAMES: usize = 4_800;
    const BASELINE_PERIODS: usize = 64;
    const RECOVERY_PERIODS: usize = 48;
    /// One engine period at 48 kHz.
    const PERIOD: Duration = Duration::from_nanos(ENGINE_FRAMES as u64 * 1_000_000_000 / 48_000);
    const RENDEZVOUS_SLICE: Duration = Duration::from_millis(50);
    const RENDEZVOUS_DEADLINE: Duration = Duration::from_secs(30);

    /// Owner clock whose time advances only on explicit test grants. Each
    /// granted wake is acknowledged by the owner parking on its next timer
    /// wait, so native-side steps never overlap a render.
    struct StepClockState {
        now: u64,
        granted: usize,
        entered: usize,
        limit: usize,
    }

    #[derive(Clone)]
    struct StepClock(Arc<(Mutex<StepClockState>, Condvar)>);

    impl StepClock {
        fn new(limit: usize) -> Self {
            Self(Arc::new((
                Mutex::new(StepClockState {
                    now: 0,
                    granted: 0,
                    entered: 0,
                    limit,
                }),
                Condvar::new(),
            )))
        }

        fn await_owner_parked(&self) {
            let (lock, signal) = &*self.0;
            let mut state = lock.lock().unwrap();
            let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
            while state.entered == 0 {
                assert!(Instant::now() < deadline, "owner never parked on its clock");
                let (guard, _) = signal
                    .wait_timeout(state, RENDEZVOUS_SLICE)
                    .expect("step clock");
                state = guard;
            }
        }

        /// Grant one schedule step and wait until the owner finished the
        /// resulting render and parked again (or stopped after the limit).
        fn advance(&self, ticks: u64) {
            let (lock, signal) = &*self.0;
            let mut state = lock.lock().unwrap();
            state.now += ticks;
            state.granted += 1;
            signal.notify_all();
            let parked = state.granted + 1;
            let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
            while state.entered < parked {
                assert!(
                    Instant::now() < deadline,
                    "owner never finished the granted period"
                );
                let (guard, _) = signal
                    .wait_timeout(state, RENDEZVOUS_SLICE)
                    .expect("step clock");
                state = guard;
            }
        }
    }

    impl VirtualRealtimeClock for StepClock {
        fn frequency(&self) -> u64 {
            48_000
        }
        fn now_ticks(&mut self) -> Result<u64, BackendError> {
            Ok(self.0.0.lock().unwrap().now)
        }
        fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
            assert_eq!(
                ticks, ENGINE_FRAMES as u64,
                "wakes must stay exactly one Graph256 period apart"
            );
            let (lock, signal) = &*self.0;
            let mut state = lock.lock().unwrap();
            state.entered += 1;
            signal.notify_all();
            let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
            while state.granted < state.entered && state.entered <= state.limit {
                assert!(Instant::now() < deadline, "owner wake was never granted");
                let (guard, _) = signal
                    .wait_timeout(state, RENDEZVOUS_SLICE)
                    .expect("step clock");
                state = guard;
            }
            Ok(if state.entered > state.limit {
                ClockWake::Stopped
            } else {
                ClockWake::Timer
            })
        }
    }

    struct StubResourceSession;
    impl VirtualResourceSession for StubResourceSession {
        fn activate(&self) -> Result<(), BackendError> {
            Ok(())
        }
        fn cancel(&self) {}
        fn status(&self) -> OwnerStatus {
            OwnerStatus::Running
        }
        fn stop_and_join(self: Box<Self>) -> Result<(), BackendError> {
            Ok(())
        }
    }

    struct PacketObservation {
        silent_frames: usize,
        native_step: usize,
        reset_epoch: u64,
        producer_epoch: u64,
        consumer_epoch: u64,
    }

    fn silent_frames(packet: &[u8]) -> usize {
        packet
            .as_chunks::<8>()
            .0
            .iter()
            .filter(|frame| {
                f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) == 0.0
                    && f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]) == 0.0
            })
            .count()
    }

    /// The native worker halves of the VAD pair, driven with the same call
    /// sequence as `run_playback`/`run_microphone` but without a driver.
    struct NativeSide {
        step: usize,
        now: FakeClock,
        playback_io: FakeIo,
        playback_tickets: Vec<PcmTicket>,
        playback_bridge: Option<PcmPlaybackBridge>,
        playback_writer: Option<AudioWriter<f32>>,
        playback_order: PacketOrder,
        playback_observed: u64,
        microphone_io: FakeIo,
        microphone_tickets: Vec<PcmTicket>,
        microphone_bridge: Option<PcmMicrophoneBridge>,
        microphone_reader: Option<AudioReader<f32>>,
        microphone_pump: Option<MicrophonePump>,
        microphone_observed: u64,
        packets: Vec<PacketObservation>,
    }

    impl NativeSide {
        fn new(playback_writer: AudioWriter<f32>, microphone_reader: AudioReader<f32>) -> Self {
            let playback_io = FakeIo {
                render_frames: NATIVE_PACKET_FRAMES,
                render_value: Some(0.25),
                ..FakeIo::default()
            };
            Self {
                step: 0,
                now: FakeClock::default(),
                playback_io,
                playback_tickets: Vec::with_capacity(crate::vad::REQUEST_SLOTS),
                playback_bridge: None,
                playback_writer: Some(playback_writer),
                playback_order: PacketOrder::default(),
                playback_observed: 0,
                microphone_io: FakeIo::default(),
                microphone_tickets: vec![],
                microphone_bridge: None,
                microphone_reader: Some(microphone_reader),
                microphone_pump: None,
                microphone_observed: 0,
                packets: vec![],
            }
        }

        /// Native time advances with owner time but on its own instant line,
        /// one period per step, so the 480-frame cadence stays rate-true.
        fn advance_native_time(&mut self) {
            self.now.advance(PERIOD);
        }

        /// The native client provides one 480-frame packet and the driver
        /// finishes one outstanding microphone WRITE every other step.
        fn complete_native_io(&mut self) {
            if self.step.is_multiple_of(2) {
                if let Some(&ticket) = self.playback_tickets.first() {
                    self.playback_io.slots[ticket.slot].ready = true;
                }
                for &ticket in &self.microphone_tickets {
                    self.microphone_io.slots[ticket.slot].ready = true;
                }
            }
            self.step += 1;
        }

        fn playback_step(&mut self, state: &SessionState) {
            if state.reset_epoch.load(Ordering::Acquire) != self.playback_observed
                && !quiesce_playback(state, &mut self.playback_observed, || {
                    self.playback_io.drain(&mut self.playback_tickets);
                    if let Some(bridge) = self.playback_bridge.take() {
                        self.playback_writer = Some(bridge.into_carrier());
                    }
                })
            {
                return;
            }
            if self.playback_bridge.is_none() {
                self.playback_bridge = Some(
                    PcmPlaybackBridge::new(
                        Channels::Stereo,
                        SampleFormat::Float32,
                        NATIVE_PACKET_FRAMES,
                        1 << 20,
                        self.playback_writer
                            .take()
                            .expect("playback carrier is free"),
                    )
                    .expect("playback bridge shape"),
                );
                self.playback_order = PacketOrder::default();
            }
            playback(
                &mut self.playback_io,
                self.playback_bridge.as_mut().expect("playback bridge"),
                &mut self.playback_tickets,
                &mut self.playback_order,
                state,
                self.playback_observed,
            )
            .expect("playback pump");
        }

        fn microphone_step(&mut self, state: &SessionState) {
            if state.reset_epoch.load(Ordering::Acquire) != self.microphone_observed {
                self.microphone_io.drain(&mut self.microphone_tickets);
                if let Some(bridge) = self.microphone_bridge.take() {
                    self.microphone_reader = Some(bridge.into_carrier());
                }
                if !quiesce_microphone(
                    state,
                    &mut self.microphone_observed,
                    self.microphone_reader
                        .as_mut()
                        .expect("microphone carrier is free"),
                ) {
                    return;
                }
                self.microphone_bridge = Some(
                    PcmMicrophoneBridge::new(
                        Channels::Stereo,
                        SampleFormat::Float32,
                        NATIVE_PACKET_FRAMES,
                        1 << 20,
                        self.microphone_reader
                            .take()
                            .expect("microphone carrier is free"),
                    )
                    .expect("microphone bridge shape"),
                );
                self.microphone_pump = Some(MicrophonePump::new(NATIVE_PACKET_FRAMES as u32));
            }
            if self.microphone_bridge.is_none() {
                // First admission of the native stream, exactly like the
                // worker building its bridge once the stream is running.
                self.microphone_bridge = Some(
                    PcmMicrophoneBridge::new(
                        Channels::Stereo,
                        SampleFormat::Float32,
                        NATIVE_PACKET_FRAMES,
                        1 << 20,
                        self.microphone_reader
                            .take()
                            .expect("microphone carrier is free"),
                    )
                    .expect("microphone bridge shape"),
                );
                self.microphone_pump = Some(MicrophonePump::new(NATIVE_PACKET_FRAMES as u32));
            }
            let before = self.microphone_io.written.len();
            microphone(
                &mut self.microphone_io,
                self.microphone_bridge.as_mut().expect("microphone bridge"),
                &mut self.microphone_tickets,
                self.microphone_pump.as_mut().expect("microphone pump"),
                state,
                self.microphone_observed,
                || self.now.now(),
            )
            .expect("microphone pump");
            let reset_epoch = state.reset_epoch.load(Ordering::Acquire);
            let producer_epoch = state.producer_quiesced_epoch.load(Ordering::Acquire);
            let consumer_epoch = state.consumer_quiesced_epoch.load(Ordering::Acquire);
            for packet in &self.microphone_io.written[before..] {
                self.packets.push(PacketObservation {
                    silent_frames: silent_frames(packet),
                    native_step: self.step,
                    reset_epoch,
                    producer_epoch,
                    consumer_epoch,
                });
            }
        }
    }

    fn passthrough_engine(
        source: VirtualPlaybackSource,
        sink: VirtualMicrophoneSink,
    ) -> (Engine<f32>, CompiledBindings, NodeId) {
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
        let mut compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: 48_000.0,
                    max_block_frames: ENGINE_FRAMES,
                    max_events_per_block: 8,
                },
                audio_byte_budget: 1 << 20,
                plan_revision: 1,
                timeline_epoch: 1,
                control_capacity: 8,
                control_horizon_frames: 48_000,
            },
        )
        .unwrap();
        let plans = compiled.engine.enable_plan_switching(1).unwrap();
        drop(plans);
        (compiled.engine, compiled.bindings, output)
    }

    #[test]
    fn one_skipped_owner_period_leaves_no_silent_microphone_packet() {
        let playback_state = Arc::new(state());
        let microphone_state = Arc::new(state());
        let (playback_writer, playback_reader) = audio_bridge(2, CARRIER_FRAMES, 1 << 20).unwrap();
        let (microphone_writer, microphone_reader) =
            audio_bridge(2, CARRIER_FRAMES, 1 << 20).unwrap();
        let source = VirtualPlaybackSource::new(playback_reader, 2, ENGINE_FRAMES, 48_000, 1 << 20)
            .unwrap()
            .with_reset_fence(
                playback_state.reset_epoch.clone(),
                playback_state.producer_quiesced_epoch.clone(),
                playback_state.consumer_quiesced_epoch.clone(),
            )
            .with_clock_drift()
            .unwrap();
        let sink = VirtualMicrophoneSink::new(microphone_writer, 2, ENGINE_FRAMES, 48_000, 1 << 20)
            .unwrap()
            .with_reset_fence(
                microphone_state.reset_epoch.clone(),
                microphone_state.producer_quiesced_epoch.clone(),
                microphone_state.consumer_quiesced_epoch.clone(),
            )
            .with_clock_drift()
            .unwrap();
        let (engine, bindings, sink_node) = passthrough_engine(source, sink);

        let total = BASELINE_PERIODS + RECOVERY_PERIODS;
        let clock = StepClock::new(total);
        let mut sessions = VirtualSessions::default();
        sessions.push_with_reset_epoch(
            Box::new(StubResourceSession),
            playback_state.reset_epoch.clone(),
        );
        sessions.push_with_reset_epoch(
            Box::new(StubResourceSession),
            microphone_state.reset_epoch.clone(),
        );
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let transport = Box::new(VirtualPreparedTransport::new(
            vec![sink_node],
            sessions,
            stop,
            gate,
            Box::new(clock.clone()),
        ));
        let owner = transport
            .attach(engine, &bindings, CancellationToken::default())
            .unwrap_or_else(|failure| panic!("virtual attachment failed: {:?}", failure.error));
        owner.activate().unwrap();
        clock.await_owner_parked();

        let mut native = NativeSide::new(playback_writer, microphone_reader);
        let skip_step = BASELINE_PERIODS;
        let mut baseline_packets = 0;
        for step in 0..total {
            if step == skip_step {
                assert_eq!(
                    playback_state.reset_epoch.load(Ordering::Acquire),
                    0,
                    "baseline must not trigger resets"
                );
                assert_eq!(microphone_state.reset_epoch.load(Ordering::Acquire), 0);
                baseline_packets = native.packets.len();
                let recent = native
                    .packets
                    .iter()
                    .rev()
                    .take(10)
                    .map(|packet| packet.silent_frames)
                    .collect::<Vec<_>>();
                assert_eq!(
                    recent.len(),
                    10,
                    "baseline must submit at least ten packets"
                );
                assert!(
                    recent.iter().all(|&silent| silent == 0),
                    "baseline never reached continuous non-zero audio: {recent:?}"
                );
            }
            native.advance_native_time();
            native.complete_native_io();
            native.playback_step(&playback_state);
            native.microphone_step(&microphone_state);
            // Delay exactly one wake by one period: backlog_periods == 2 and
            // the debt ledger repays the missed block on the immediate redue.
            let ticks = if step == skip_step {
                (ENGINE_FRAMES * 2) as u64
            } else {
                ENGINE_FRAMES as u64
            };
            clock.advance(ticks);
        }

        let returned = owner.stop_and_join();
        assert!(returned.error.is_none());
        // The timeline must equal the granted wall-clock periods exactly: the
        // skipped period is repaid, not discarded (the discarded-skip design
        // ran a permanent frame deficit on both carriers at once).
        assert_eq!(
            returned.engine.timeline(),
            ((total + 1) * ENGINE_FRAMES) as u64
        );

        // The single skipped period is absorbed as jitter: no boundary reset
        // was requested, no fence moved, and the packet stream stayed audible.
        assert_eq!(
            playback_state.reset_epoch.load(Ordering::Acquire),
            0,
            "one skipped period must not reset the playback boundary"
        );
        assert_eq!(
            microphone_state.reset_epoch.load(Ordering::Acquire),
            0,
            "one skipped period must not reset the microphone boundary"
        );
        assert_eq!(
            microphone_state
                .producer_quiesced_epoch
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            microphone_state
                .consumer_quiesced_epoch
                .load(Ordering::Acquire),
            0
        );
        // Steady state legitimately keeps four playback READs and one
        // microphone WRITE in flight; the reset must not orphan book slots.
        assert_eq!(
            native.playback_io.book.pending(),
            native.playback_tickets.len(),
            "every playback book slot must belong to a live request"
        );
        assert_eq!(
            native.microphone_io.book.pending(),
            native.microphone_tickets.len(),
            "every microphone book slot must belong to a live request"
        );

        let post: Vec<(usize, usize, u64, u64, u64)> = native.packets[baseline_packets..]
            .iter()
            .map(|packet| {
                (
                    packet.native_step,
                    packet.silent_frames,
                    packet.reset_epoch,
                    packet.producer_epoch,
                    packet.consumer_epoch,
                )
            })
            .collect();
        let silent: Vec<_> = post.iter().filter(|(_, silent, ..)| *silent > 0).collect();
        assert!(
            silent.is_empty(),
            "a single skipped owner period must not silence any microphone \
             packet (target signature: first post-reset packet is 0/480); \
             post-skip packets (native_step, silent_frames, reset_epoch, \
             producer_epoch, consumer_epoch): {post:?}"
        );
    }

    /// The redesign's master invariant under sustained jitter: every 32nd
    /// wake arrives two periods late (the target trace showed twelve
    /// skipped=1 events plus multi-period gaps inside 45 s), the native pair
    /// keeps streaming through the injected stalls, and the debt ledger
    /// repays every skipped period. Over 2048 wakes this pins: the engine
    /// timeline equals the granted wall-clock frames exactly (the discarded-
    /// skip design ran a permanent -1791 ppm deficit), no boundary epoch ever
    /// moves, no microphone packet carries a silent tail after warm-up, both
    /// carrier water levels stay well inside the ring (no starvation, no
    /// overflow, debt never pinned), and no book slot is orphaned.
    #[test]
    fn periodic_owner_skips_are_repaid_with_bounded_levels_and_zero_silent_packets() {
        let playback_state = Arc::new(state());
        let microphone_state = Arc::new(state());
        let (playback_writer, playback_reader) = audio_bridge(2, CARRIER_FRAMES, 1 << 20).unwrap();
        let (microphone_writer, microphone_reader) =
            audio_bridge(2, CARRIER_FRAMES, 1 << 20).unwrap();
        let playback_probe = Arc::new(BoundaryProbe::default());
        let microphone_probe = Arc::new(BoundaryProbe::default());
        let source = VirtualPlaybackSource::new(playback_reader, 2, ENGINE_FRAMES, 48_000, 1 << 20)
            .unwrap()
            .with_reset_fence(
                playback_state.reset_epoch.clone(),
                playback_state.producer_quiesced_epoch.clone(),
                playback_state.consumer_quiesced_epoch.clone(),
            )
            .with_probe(playback_probe.clone())
            .with_clock_drift()
            .unwrap();
        let sink = VirtualMicrophoneSink::new(microphone_writer, 2, ENGINE_FRAMES, 48_000, 1 << 20)
            .unwrap()
            .with_reset_fence(
                microphone_state.reset_epoch.clone(),
                microphone_state.producer_quiesced_epoch.clone(),
                microphone_state.consumer_quiesced_epoch.clone(),
            )
            .with_probe(microphone_probe.clone())
            .with_clock_drift()
            .unwrap();
        let (engine, bindings, sink_node) = passthrough_engine(source, sink);

        const TOTAL_STEPS: usize = 2_048;
        const SKIP_INTERVAL: usize = 32;
        const SKIP_EXTRA_PERIODS: usize = 2;
        const WARMUP_STEPS: usize = 48;
        let skips = (1..TOTAL_STEPS)
            .filter(|step| step % SKIP_INTERVAL == 0)
            .count();
        let wall_periods = TOTAL_STEPS + skips * SKIP_EXTRA_PERIODS;
        let clock = StepClock::new(TOTAL_STEPS);
        let mut sessions = VirtualSessions::default();
        sessions.push_with_reset_epoch(
            Box::new(StubResourceSession),
            playback_state.reset_epoch.clone(),
        );
        sessions.push_with_reset_epoch(
            Box::new(StubResourceSession),
            microphone_state.reset_epoch.clone(),
        );
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let transport = Box::new(VirtualPreparedTransport::new(
            vec![sink_node],
            sessions,
            stop,
            gate,
            Box::new(clock.clone()),
        ));
        let owner = transport
            .attach(engine, &bindings, CancellationToken::default())
            .unwrap_or_else(|failure| panic!("virtual attachment failed: {:?}", failure.error));
        owner.activate().unwrap();
        clock.await_owner_parked();

        let mut native = NativeSide::new(playback_writer, microphone_reader);
        let mut native_substeps = 0usize;
        let (mut playback_low, mut playback_high) = (u32::MAX, 0u32);
        let (mut microphone_low, mut microphone_high) = (u32::MAX, 0u32);
        for step in 0..TOTAL_STEPS {
            let late = usize::from(step > 0 && step % SKIP_INTERVAL == 0) * SKIP_EXTRA_PERIODS;
            // The native pair streams through the injected stall: each extra
            // wall period advances the native side one sub-step before the
            // late owner wake repays it from the ledger.
            for _ in 0..=late {
                native.advance_native_time();
                native.complete_native_io();
                native.playback_step(&playback_state);
                native.microphone_step(&microphone_state);
                native_substeps += 1;
            }
            clock.advance((ENGINE_FRAMES * (1 + late)) as u64);
            if step >= WARMUP_STEPS {
                let playback = playback_probe.snapshot().level_frames;
                playback_low = playback_low.min(playback);
                playback_high = playback_high.max(playback);
                let microphone = microphone_probe.snapshot().level_frames;
                microphone_low = microphone_low.min(microphone);
                microphone_high = microphone_high.max(microphone);
            }
        }

        let returned = owner.stop_and_join();
        assert!(returned.error.is_none());
        assert_eq!(
            returned.engine.timeline(),
            (wall_periods * ENGINE_FRAMES) as u64,
            "every granted wall-clock period must render exactly once"
        );
        assert_eq!(
            playback_state.reset_epoch.load(Ordering::Acquire),
            0,
            "jitter must never escalate to a boundary reset"
        );
        assert_eq!(microphone_state.reset_epoch.load(Ordering::Acquire), 0);
        assert_eq!(
            microphone_state
                .producer_quiesced_epoch
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            microphone_state
                .consumer_quiesced_epoch
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            native.playback_io.book.pending(),
            native.playback_tickets.len(),
            "every playback book slot must belong to a live request"
        );
        assert_eq!(
            native.microphone_io.book.pending(),
            native.microphone_tickets.len(),
            "every microphone book slot must belong to a live request"
        );
        let warmup_packets = native
            .packets
            .iter()
            .take_while(|packet| packet.native_step <= WARMUP_STEPS)
            .count();
        let post: Vec<(usize, usize)> = native.packets[warmup_packets..]
            .iter()
            .map(|packet| (packet.native_step, packet.silent_frames))
            .collect();
        let silent: Vec<_> = post.iter().filter(|(_, silent)| *silent > 0).collect();
        assert!(
            silent.is_empty(),
            "periodic jitter must not silence any microphone packet;              (native_step, silent_frames): {post:?}"
        );
        assert!(
            post.len() >= TOTAL_STEPS / 3,
            "the native cadence must keep producing packets: {} packets over {} sub-steps",
            post.len(),
            native_substeps,
        );
        // The band keeps more than a packet and a half away from both rails.
        // The microphone starts near empty (the native stream begins with the
        // graph) and rebuilds toward its servo target at the bounded ±2000 ppm
        // pace - that slow rebuild is the designed behavior, not drift.
        assert!(
            playback_low > 400 && playback_high < CARRIER_FRAMES as u32 - 400,
            "playback level must stay inside the ring under sustained jitter:              {playback_low}..{playback_high}"
        );
        assert!(
            microphone_low > 400 && microphone_high < CARRIER_FRAMES as u32 - 400,
            "microphone level must stay inside the ring under sustained jitter:              {microphone_low}..{microphone_high}"
        );
        assert_eq!(
            playback_probe.snapshot().xruns + microphone_probe.snapshot().xruns,
            0,
            "no starvation or overflow may occur under sustained jitter"
        );
    }
}
