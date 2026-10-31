use super::*;
use crate::vad::provider::telemetry::{PublicationIdentity, publication::set_test_ticks};

fn bind_publication(f: &Fixture) {
    f.state.telemetry.publication.bind(PublicationIdentity {
        native_stamp: [11, 7, 11, 3, 5],
        reset_epoch: 0,
        admission_ticket: f.state.admitted_ticket.load(Ordering::Acquire),
        active_revision: 9,
        format: SampleFormat::Float32 as u32,
        channels: 2,
        packet_frames: 1,
    });
}

#[derive(Clone, Copy, Debug)]
enum Change {
    Revoke,
    Reset,
    Cancel,
    Drain,
    Deactivate,
    Ticket,
    Revision,
}

fn change(state: &SessionState, change: Change) {
    match change {
        Change::Revoke => state.revoke(),
        Change::Reset => state.reset_epoch.store(1, Ordering::Release),
        Change::Cancel => state.cancel.store(true, Ordering::Release),
        Change::Drain => state.drained.store(true, Ordering::Release),
        Change::Deactivate => state.active.store(false, Ordering::Release),
        Change::Ticket | Change::Revision => {
            let revision = if matches!(change, Change::Ticket) {
                9
            } else {
                10
            };
            let ticket = state.admission.set(Some(revision));
            state.admitted_ticket.store(ticket, Ordering::Release);
            state.admitted_revision.store(revision, Ordering::Release);
        }
    }
}

fn inject(f: &Fixture, change_kind: Change, before_write: bool) {
    let state = f.state.clone();
    let hook: Box<dyn FnOnce()> = Box::new(move || change(&state, change_kind));
    if before_write {
        BEFORE_DECODE_CALL.with(|slot| *slot.borrow_mut() = Some(hook));
    } else {
        AFTER_PLAYBACK_PUBLISH.with(|slot| *slot.borrow_mut() = Some(hook));
    }
}

#[test]
fn input_capture_successful_write_revoked_during_decode_is_not_unqualified_publication() {
    let mut f = Fixture::new(8, 8);
    bind_publication(&f);
    let state = f.state.clone();
    AFTER_PLAYBACK_PUBLISH.with(|hook| *hook.borrow_mut() = Some(Box::new(move || state.revoke())));
    f.packet(1, [0.5, -0.5]);
    assert_eq!(f.pump().unwrap(), PumpState::Current);
    let mut samples = [0.; 2];
    assert_eq!(
        f.reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(samples, [0.5, -0.5], "the real write must not be erased");
    let publication = f.state.telemetry_snapshot().playback_publication.unwrap();
    assert_eq!(publication.successful_decode_calls, 1);
    assert_eq!(publication.ambiguous_packets, 1);
    let artifact = f.finish();
    assert_eq!(artifact.stored_frames, 1);
    assert_eq!(
        artifact.packets[0].publication,
        "published_identity_changed"
    );
    assert_eq!(artifact.seal_reason, "identity_changed_during_publication");
}

#[test]
fn input_capture_identity_races_preserve_audio_and_capacity_seals() {
    for change_kind in [
        Change::Revoke,
        Change::Reset,
        Change::Cancel,
        Change::Drain,
        Change::Deactivate,
        Change::Ticket,
        Change::Revision,
    ] {
        for before_write in [false, true] {
            for frames in [1, 8] {
                let mut f = Fixture::new(frames, 8);
                bind_publication(&f);
                let original_ticket = f.state.admitted_ticket.load(Ordering::Acquire);
                inject(&f, change_kind, before_write);
                f.packet(1, [0.25, -0.5]);
                assert_eq!(
                    f.pump().unwrap(),
                    if matches!(change_kind, Change::Reset) {
                        PumpState::Reset
                    } else {
                        PumpState::Current
                    }
                );
                let mut samples = [0.; 2];
                assert_eq!(
                    f.reader
                        .read_interleaved(&mut samples)
                        .unwrap()
                        .transferred_frames,
                    1
                );
                assert_eq!(samples, [0.25, -0.5]);
                let telemetry = f.state.telemetry_snapshot();
                let publication = telemetry.playback_publication.unwrap();
                assert_eq!(publication.successful_decode_calls, 1);
                assert_eq!(publication.ambiguous_packets, 1);
                assert_eq!(telemetry.carrier_dropped_packets, 0);
                let artifact = f.finish();
                assert_eq!(
                    artifact.packets[0].publication, "published_identity_changed",
                    "{change_kind:?} before={before_write} frames={frames}"
                );
                assert_eq!(artifact.identity.unwrap().admission_ticket, original_ticket);
                assert_eq!(artifact.identity.unwrap().active_revision, 9);
                assert_eq!(
                    artifact.seal_reason,
                    if frames == 1 {
                        "frame_capacity"
                    } else {
                        "identity_changed_during_publication"
                    }
                );
                assert_eq!(
                    artifact.raw,
                    [0.25_f32, -0.5]
                        .into_iter()
                        .flat_map(f32::to_le_bytes)
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn input_capture_raced_success_seals_before_later_pcm_even_if_admission_is_restored() {
    let mut f = Fixture::new(8, 8);
    bind_publication(&f);
    f.packet(1, [1.; 2]);
    f.pump().unwrap();
    inject(&f, Change::Revoke, false);
    f.packet(2, [2.; 2]);
    f.pump().unwrap();
    // Even restoring these mutable fields cannot reopen a sealed capture.
    f.state.admitted_revision.store(9, Ordering::Release);
    f.packet(3, [3.; 2]);
    f.pump().unwrap();
    let mut samples = [0.; 6];
    assert_eq!(
        f.reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        3
    );
    assert_eq!(samples, [1., 1., 2., 2., 3., 3.]);
    let artifact = f.finish();
    assert_eq!(artifact.stored_frames, 2);
    assert_eq!(artifact.packets[0].publication, "published");
    assert_eq!(
        artifact.packets[1].publication,
        "published_identity_changed"
    );
    assert_eq!(artifact.seal_reason, "identity_changed_during_publication");
}

#[test]
fn input_capture_identity_race_never_hides_carrier_full_or_decode_error() {
    for malformed in [false, true] {
        let mut f = Fixture::new(8, if malformed { 8 } else { 1 });
        bind_publication(&f);
        f.packet(1, [1.; 2]);
        f.pump().unwrap();
        f.packet(2, [2.; 2]);
        if malformed {
            f.io.slots[f.tickets[0].slot].packet.pop();
        }
        inject(&f, Change::Revoke, false);
        if malformed {
            assert!(f.pump().is_err());
        } else {
            assert_eq!(f.pump().unwrap(), PumpState::Reset);
        }
        let publication = f.state.telemetry_snapshot().playback_publication.unwrap();
        assert_eq!(publication.successful_decode_calls, 1);
        assert_eq!(publication.decode_error_packets, u64::from(malformed));
        assert_eq!(publication.carrier_full_packets, u64::from(!malformed));
        let artifact = f.finish();
        assert_eq!(artifact.packets[0].publication, "published");
        assert_eq!(
            artifact.packets[1].publication,
            if malformed {
                "decode_error"
            } else {
                "carrier_full"
            }
        );
    }
}

#[test]
fn input_capture_owner_checks_do_not_require_a_valid_aggregate_publication_history() {
    for bound in [false, true] {
        let mut f = Fixture::new(8, 8);
        if bound {
            bind_publication(&f);
            f.state.telemetry.publication.invalidate();
        }
        f.packet(1, [1.; 2]);
        assert_eq!(f.pump().unwrap(), PumpState::Current);
        let artifact = f.finish();
        assert_eq!(artifact.packets[0].publication, "published");
        assert_eq!(artifact.seal_reason, "worker_exit");
    }
}

#[test]
fn input_capture_race_checks_and_sealing_do_not_allocate_or_free_in_pump() {
    let mut f = Fixture::new(8, 8);
    bind_publication(&f);
    f.io.collected.reserve(crate::vad::REQUEST_SLOTS);
    f.io.defer_reads = 1;
    f.packet(1, [1.; 2]);
    let result = crate::test_allocation::assert_allocates_nothing(|| f.pump());
    assert_eq!(result.unwrap(), PumpState::Current);
    // The injected Box<FnOnce> hooks themselves free test instrumentation.
    // Check the raced diagnostic transition separately from those hooks.
    crate::test_allocation::assert_allocates_nothing(|| {
        f.capture
            .as_mut()
            .unwrap()
            .published(Some(0), "published", false);
    });
    assert_eq!(
        f.finish().packets[0].publication,
        "published_identity_changed"
    );
}

#[test]
fn input_capture_enabled_disabled_identity_races_keep_actual_pump_behavior_equal() {
    for change_kind in [
        Change::Revoke,
        Change::Reset,
        Change::Cancel,
        Change::Drain,
        Change::Deactivate,
        Change::Ticket,
        Change::Revision,
    ] {
        for before_write in [false, true] {
            let mut enabled = Fixture::new(8, 8);
            let mut disabled = Fixture::new(8, 8);
            disabled.capture.take();
            for f in [&mut enabled, &mut disabled] {
                bind_publication(f);
                f.packet(1, [0.25, -0.5]);
                f.io.slots[f.tickets[0].slot].stamp.qpc = u64::MAX;
                set_test_ticks(&[Some(100), Some(110)]);
                inject(f, change_kind, before_write);
                let result = f.pump().unwrap();
                assert_eq!(
                    result,
                    if matches!(change_kind, Change::Reset) {
                        PumpState::Reset
                    } else {
                        PumpState::Current
                    }
                );
            }
            let mut left = [0.; 2];
            let mut right = [0.; 2];
            assert_eq!(
                enabled.reader.read_interleaved(&mut left).unwrap(),
                disabled.reader.read_interleaved(&mut right).unwrap()
            );
            assert_eq!(left, right);
            assert_eq!(enabled.io.reads, disabled.io.reads);
            assert_eq!(enabled.io.collected, disabled.io.collected);
            assert_eq!(enabled.io.written, disabled.io.written);
            assert_eq!(
                enabled.tickets.iter().map(|t| t.slot).collect::<Vec<_>>(),
                disabled.tickets.iter().map(|t| t.slot).collect::<Vec<_>>()
            );
            assert_eq!(
                enabled.state.telemetry_snapshot(),
                disabled.state.telemetry_snapshot()
            );
            assert_eq!(
                enabled.state.reset_epoch.load(Ordering::Acquire),
                disabled.state.reset_epoch.load(Ordering::Acquire)
            );
        }
    }
}
