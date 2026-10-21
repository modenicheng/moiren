use super::*;
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
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            now: Cell::new(Instant::now()),
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
}

thread_local! {
    static BEFORE_PLAYBACK_PUBLISH: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn before_playback_publish() {
    BEFORE_PLAYBACK_PUBLISH.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
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
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), BackendError> {
        self.book.check(ticket).unwrap();
        self.waits.push((ticket, timeout));
        if self.signal_on_wait {
            self.slots[ticket.slot].ready = true;
        }
        Ok(())
    }
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, BackendError> {
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
    fn read(&mut self) -> Result<PcmTicket, BackendError> {
        let ticket = self.book.reserve().unwrap();
        self.reads += 1;
        self.slots[ticket.slot] = Slot {
            ready: false,
            stamp: PcmPacketStamp {
                sequence: self.reads,
                qpc: self.reads,
            },
            packet: [self.reads as f32; 2]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
            capture: false,
            wrong_shape: false,
            dropped: false,
        };
        Ok(ticket)
    }
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, BackendError> {
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
        Instant::now,
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
            Instant::now
        )
        .unwrap(),
        PumpState::Reset
    );
    assert_eq!(io.written.len(), 1);
    io.drain(&mut tickets);
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = Some(bridge);
    let mut reader = None;
    super::super::teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
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
        Instant::now,
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
    super::super::teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
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
    assert_eq!(
        tickets.len(),
        1,
        "completion immediately replenishes capture"
    );
    assert_eq!(io.written.len(), 2);
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

    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        Instant::now,
    )
    .unwrap();
    assert_eq!(tickets.len(), 1, "keep only one native WRITE pending");
    assert_eq!(io.written.len(), 1);

    microphone(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut pump,
        &state,
        0,
        Instant::now,
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
        Instant::now,
    )
    .unwrap();
    assert_eq!(
        tickets.len(),
        1,
        "completion immediately replenishes capture"
    );
    assert_eq!(io.written.len(), 2);
    for (packet, value) in io.written.iter().zip([1.0_f32, 2.0]) {
        assert_eq!(packet, &value.to_le_bytes().repeat(2));
    }
}
#[test]
fn microphone_completion_replenishes_before_a_delayed_worker_reschedule() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 1);

    f.collect(false);
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.tickets.len(), 1);

    // The native timer can run while the non-RT worker is not scheduled. Its
    // next capture callback must still have the replacement WRITE available.
    f.clock.advance(Duration::from_millis(12));
    f.cycle().unwrap();
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.tickets.len(), 1);
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
        assert_eq!(self.tickets.len(), 1);
    }
}

#[test]
fn microphone_observation_gap_resets_before_collecting_or_rearming_old_pcm() {
    let mut f = MicrophoneFixture::new(480);
    f.state.active.store(true, Ordering::Release);
    let admitted = f.state.admission.set(Some(9));
    f.state.admitted_ticket.store(admitted, Ordering::Release);
    f.state.admitted_revision.store(9, Ordering::Release);
    f.cycle().unwrap();
    let pending = f.tickets[0];
    f.io.slots[pending.slot].ready = true;
    // A shallow, lossless publication followed by a stopped producer: no full
    // carrier notification can protect this old packet from a worker stall.
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(100));
    assert_eq!(f.cycle().unwrap(), PumpState::Reset);
    assert!(
        f.io.collected.is_empty(),
        "reset before collecting completion"
    );
    assert_eq!(f.tickets, vec![pending]);
    assert_eq!(
        f.io.written.len(),
        1,
        "the reset must not create another submission"
    );
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
    assert_eq!(f.cycle().unwrap(), PumpState::Reset);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
    assert!(
        f.state.admitted(),
        "a timeline gap must not revoke admission"
    );
    assert!(f.state.failure.lock().unwrap().is_none());

    f.io.drain(&mut f.tickets);
    let mut reader = f.bridge.into_carrier();
    let mut observed = 0;
    assert!(!quiesce_microphone(&f.state, &mut observed, &mut reader));
    assert_eq!(reader.available_frames(), 480);
    // The fake graph producer has stopped; acknowledge only after its last A.
    f.state.producer_quiesced_epoch.store(1, Ordering::Release);
    assert!(quiesce_microphone(&f.state, &mut observed, &mut reader));
    assert_eq!(reader.available_frames(), 0);
    f.writer.write_interleaved_exact(&vec![5.; 960]).unwrap();
    let mut bridge = PcmMicrophoneBridge::new(
        Channels::Stereo,
        SampleFormat::Float32,
        480,
        131_072,
        reader,
    )
    .unwrap();
    assert_eq!(
        microphone(
            &mut f.io,
            &mut bridge,
            &mut f.tickets,
            &mut MicrophonePump::new(480),
            &f.state,
            observed,
            || f.clock.now(),
        )
        .unwrap(),
        PumpState::Current
    );
    assert_eq!(f.io.written[1], 5.0_f32.to_le_bytes().repeat(960));
}

#[test]
fn microphone_observation_gap_during_phase_cannot_encode_shallow_old_pcm() {
    let mut f = MicrophoneFixture::new(480);
    f.cycle().unwrap();
    f.collect(false);
    f.writer.write_interleaved_exact(&vec![1.; 960]).unwrap();
    f.clock.advance(Duration::from_millis(100));
    assert_eq!(f.cycle().unwrap(), PumpState::Reset);
    assert_eq!(f.io.written.len(), 2);
    assert_eq!(f.tickets.len(), 1);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
}

#[test]
fn microphone_observation_gap_allows_one_native_period_plus_io_wait() {
    for (frames, bound) in [
        (48, Duration::from_millis(11)),
        (49, Duration::from_nanos(11_020_834)),
        (480, Duration::from_millis(20)),
        (4800, Duration::from_millis(110)),
    ] {
        let mut f = MicrophoneFixture::new(frames);
        f.cycle().unwrap();
        f.clock.advance(bound);
        assert_eq!(f.cycle().unwrap(), PumpState::Current);
        assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
        f.clock.advance(bound + Duration::from_nanos(1));
        assert_eq!(f.cycle().unwrap(), PumpState::Reset);
        assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
    }
}

#[test]
fn microphone_observation_gap_immediately_before_completion_leaves_ticket_pending() {
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
        PumpState::Reset
    );
    assert!(f.io.collected.is_empty());
    assert_eq!(f.tickets, vec![pending]);
    assert_eq!(f.io.book.pending(), 1);
}

#[test]
fn microphone_observation_gap_inside_completion_cannot_rearm_phase() {
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
        PumpState::Reset
    );
    assert_eq!(f.io.collected.len(), 1);
    assert_eq!(f.io.written.len(), 1);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
    f.io.drain(&mut f.tickets);
    assert_eq!(f.io.book.pending(), 0);
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
