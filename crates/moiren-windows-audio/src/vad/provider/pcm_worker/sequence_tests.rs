use super::*;

fn playback_batch(io: &mut FakeIo, tickets: &[PcmTicket], stamps: &[(u64, u64)]) {
    for (&ticket, &(sequence, qpc)) in tickets.iter().zip(stamps) {
        io.slots[ticket.slot].ready = true;
        io.slots[ticket.slot].stamp = PcmPacketStamp { sequence, qpc };
    }
}

fn assert_gaps(state: &SessionState, observations: u64, slots: u64) {
    let snapshot = serde_json::to_value(state.telemetry.snapshot()).unwrap();
    assert_eq!(snapshot["sequence_gap_observations"], observations);
    assert_eq!(snapshot["missing_sequence_slots"], slots);
}

#[test]
fn packet_order_classifies_first_contiguous_gap_and_stale_without_wrapping() {
    let mut order = PacketOrder::default();
    let stamp = |sequence, qpc| PcmPacketStamp { sequence, qpc };
    assert_eq!(order.accept(0, stamp(0, 10)), PacketProgress::First);
    assert_eq!(order.accept(0, stamp(1, 10)), PacketProgress::Contiguous);
    assert_eq!(order.accept(0, stamp(1, 11)), PacketProgress::Stale);
    assert_eq!(order.accept(0, stamp(9, 9)), PacketProgress::Stale);
    assert_eq!(
        order.accept(0, stamp(4, 10)),
        PacketProgress::Gap { missing_slots: 2 }
    );
    assert_eq!(order.accept(1, stamp(0, 1)), PacketProgress::First);
    assert_eq!(
        order.accept(1, stamp(u64::MAX, 1)),
        PacketProgress::Gap {
            missing_slots: u64::MAX - 1
        }
    );
    assert_eq!(order.accept(1, stamp(0, 2)), PacketProgress::Stale);
    assert_eq!(order.accept(1, stamp(u64::MAX, 2)), PacketProgress::Stale);
    assert_eq!(
        order.accept(2, stamp(u64::MAX - 1, 1)),
        PacketProgress::First
    );
    assert_eq!(
        order.accept(2, stamp(u64::MAX, 1)),
        PacketProgress::Contiguous
    );
}

#[test]
fn playback_sequence_gaps_continue_without_reset_or_invented_signal_drops() {
    let state = state();
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, mut reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    playback_batch(
        &mut io,
        &tickets,
        &[(10, 100), (13, 100), (17, 101), (18, 102)],
    );
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
        PumpState::Current
    );
    assert_gaps(&state, 2, 5);
    let snapshot = state.telemetry.snapshot();
    assert_eq!(snapshot.completed_packets, 4);
    assert_eq!(snapshot.dropped_packets, 0);
    assert_eq!(snapshot.stale_packets, 0);
    assert_eq!(state.reset_epoch.load(Ordering::Acquire), 0);
    let mut samples = [0.0; 8];
    assert_eq!(
        reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        4
    );
    assert_eq!(samples, [1., 1., 2., 2., 3., 3., 4., 4.]);
}

#[test]
fn playback_sequence_stale_stamps_do_not_poison_the_accepted_frontier() {
    let state = state();
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    playback_batch(
        &mut io,
        &tickets,
        &[(10, 100), (50, 99), (9, 101), (13, 100)],
    );
    playback(
        &mut io,
        &mut bridge,
        &mut tickets,
        &mut PacketOrder::default(),
        &state,
        0,
    )
    .unwrap();
    assert_gaps(&state, 1, 2);
    assert_eq!(state.telemetry.snapshot().stale_packets, 2);
}

#[test]
fn playback_sequence_new_epoch_cannot_compare_with_an_old_sequence_prefix() {
    let state = state();
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    playback_batch(&mut io, &tickets, &[(90, 900)]);
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap();
    // Keep the order object deliberately: epoch identity must itself fence
    // diagnostic comparisons, not rely on a caller remembering to replace it.
    io.drain(&mut tickets);
    state.reset_epoch.store(1, Ordering::Release);
    state.producer_quiesced_epoch.store(1, Ordering::Release);
    state.consumer_quiesced_epoch.store(1, Ordering::Release);
    tickets = pending_reads(&mut io);
    playback_batch(&mut io, &tickets, &[(1, 10), (4, 11)]);
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 1).unwrap();
    assert_gaps(&state, 1, 2);
    assert_eq!(state.telemetry.snapshot().stale_packets, 0);
}

#[test]
fn playback_sequence_reset_before_publication_cannot_record_a_false_gap() {
    let state = Arc::new(state());
    let mut io = FakeIo::default();
    let mut tickets = pending_reads(&mut io);
    let (writer, _reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Stereo, SampleFormat::Float32, 1, 4096, writer).unwrap();
    let mut order = PacketOrder::default();
    playback_batch(&mut io, &tickets, &[(1, 10)]);
    playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap();
    playback_batch(&mut io, &tickets, &[(5, 11)]);
    let reset = state.clone();
    BEFORE_PLAYBACK_PUBLISH.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            reset.reset_epoch.store(1, Ordering::Release);
        }));
    });
    assert_eq!(
        playback(&mut io, &mut bridge, &mut tickets, &mut order, &state, 0).unwrap(),
        PumpState::Reset
    );
    assert_gaps(&state, 0, 0);
}

#[test]
fn microphone_sequence_holes_are_host_submissions_not_native_missing_slots() {
    let mut fixture = MicrophoneFixture::new(480);
    fixture.cycle().unwrap();
    fixture.collect(false);
    fixture.clock.advance(Duration::from_millis(5));
    fixture.cycle().unwrap();
    let slot = fixture.tickets[0].slot;
    fixture.io.slots[slot].stamp.sequence = 100;
    fixture.collect(false);
    assert_gaps(&fixture.state, 0, 0);
    assert_eq!(fixture.state.reset_epoch.load(Ordering::Acquire), 0);
}
