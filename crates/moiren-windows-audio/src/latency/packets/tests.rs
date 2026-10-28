use super::*;

fn sample(sequence: u64) -> PacketPublicationSample {
    PacketPublicationSample {
        offered_frames: 12,
        accepted_frames: 8,
        qpc_before_100ns: Some(sequence * 10),
        qpc_after_100ns: Some(sequence * 10 + 1),
        origin: PacketPublicationOrigin::NativeCapture,
        buffer_generation: sequence / 3,
        flags: (sequence % 8) as u32,
    }
}

#[test]
fn empty_history_and_zero_offer_have_no_publication() {
    let (mut writer, observer) = packet_history();
    let empty = observer.snapshot().unwrap();
    assert_eq!(empty.total_events, 0);
    assert_eq!(empty.overwritten_events, 0);
    assert_eq!(empty.offered_frames, 0);
    assert_eq!(empty.published_frames, 0);
    assert_eq!(empty.unavailable_clocks, 0);
    assert_eq!(empty.regressed_clocks, 0);
    assert!(!empty.saturated);
    assert!(empty.events.is_empty());
    writer.record(PacketPublicationSample::default());
    assert_eq!(observer.snapshot().unwrap(), empty);
}

#[test]
fn full_partial_and_rejected_publications_keep_exact_conservation_frontiers() {
    let (mut writer, observer) = packet_history();
    for (index, accepted) in [12, 8, 0].into_iter().enumerate() {
        writer.record(PacketPublicationSample {
            accepted_frames: accepted,
            ..sample(index as u64 + 1)
        });
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 3);
    assert_eq!(snapshot.offered_frames, 36);
    assert_eq!(snapshot.published_frames, 20);
    assert_eq!(snapshot.offered_frames - snapshot.published_frames, 16);
    assert!(!snapshot.saturated);
    let mut offered = 0;
    let mut published = 0;
    for (index, event) in snapshot.events.iter().enumerate() {
        assert_eq!(event.sequence, index as u64 + 1);
        assert_eq!(event.offered_before_frames, offered);
        assert_eq!(event.published_before_frames, published);
        offered += event.sample.offered_frames;
        published += event.sample.accepted_frames;
    }
    assert_eq!(snapshot.events[2].sample.accepted_frames, 0);
    assert_eq!(offered, snapshot.offered_frames);
    assert_eq!(published, snapshot.published_frames);
}

#[test]
fn invalid_acceptance_preserves_prefix_and_sticks_saturation() {
    for offered in [0, 12] {
        let (mut writer, observer) = packet_history();
        writer.record(sample(1));
        let prefix = observer.snapshot().unwrap();
        writer.record(PacketPublicationSample {
            offered_frames: offered,
            accepted_frames: offered + 1,
            ..sample(2)
        });
        writer.record(sample(3));
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.saturated);
        assert_eq!(snapshot.events, prefix.events);
        assert_eq!(snapshot.total_events, prefix.total_events);
        assert_eq!(snapshot.offered_frames, prefix.offered_frames);
        assert_eq!(snapshot.published_frames, prefix.published_frames);
    }
}

#[test]
fn retained_suffix_keeps_cumulative_frontiers_without_imaginary_initial_fill() {
    let (mut writer, observer) = packet_history();
    for sequence in 1..=259 {
        writer.record(sample(sequence));
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 259);
    assert_eq!(snapshot.overwritten_events, 3);
    assert_eq!(snapshot.events.len(), 256);
    assert_eq!(snapshot.offered_frames, 259 * 12);
    assert_eq!(snapshot.published_frames, 259 * 8);
    for (index, event) in snapshot.events.iter().enumerate() {
        assert_eq!(event.sequence, index as u64 + 4);
        assert_eq!(event.offered_before_frames, (event.sequence - 1) * 12);
        assert_eq!(event.published_before_frames, (event.sequence - 1) * 8);
        assert_eq!(event.sample, sample(event.sequence));
    }
    assert_eq!(snapshot.events[0].published_before_frames, 24);
    assert!(!snapshot.saturated);
}

#[test]
fn missing_or_zero_boundaries_are_unknown_once_per_event() {
    let (mut writer, observer) = packet_history();
    let clocks = [
        (Some(100), Some(110)),
        (None, Some(120)),
        (Some(130), None),
        (Some(0), Some(0)),
        (None, None),
    ];
    for (index, (before, after)) in clocks.into_iter().enumerate() {
        writer.record(PacketPublicationSample {
            qpc_before_100ns: before,
            qpc_after_100ns: after,
            ..sample(index as u64 + 1)
        });
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.unavailable_clocks, 4);
    assert_eq!(snapshot.regressed_clocks, 0);
    assert_eq!(snapshot.events[1].sample.qpc_before_100ns, None);
    assert_eq!(snapshot.events[2].sample.qpc_after_100ns, None);
    assert_eq!(snapshot.events[3].sample.qpc_before_100ns, None);
    assert_eq!(snapshot.events[3].sample.qpc_after_100ns, None);
    assert!(!snapshot.saturated);
}

#[test]
fn missing_boundaries_do_not_conceal_regressions_and_stamps_are_not_repaired() {
    let (mut writer, observer) = packet_history();
    let clocks = [
        (Some(100), Some(110)),
        (None, None),
        (None, Some(90)),
        (Some(80), None),
        (Some(70), Some(60)),
        (Some(200), Some(190)),
        (Some(190), Some(190)),
    ];
    for (index, (before, after)) in clocks.into_iter().enumerate() {
        writer.record(PacketPublicationSample {
            qpc_before_100ns: before,
            qpc_after_100ns: after,
            ..sample(index as u64 + 1)
        });
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.unavailable_clocks, 3);
    assert_eq!(snapshot.regressed_clocks, 4);
    assert_eq!(snapshot.events[4].sample.qpc_before_100ns, Some(70));
    assert_eq!(snapshot.events[4].sample.qpc_after_100ns, Some(60));
    assert!(!snapshot.saturated);
}

#[test]
fn origin_generation_and_native_flags_roundtrip_without_time_interpretation() {
    let (mut writer, observer) = packet_history();
    assert_eq!(
        PacketPublicationSample::default().origin,
        PacketPublicationOrigin::Unknown
    );
    for (index, origin) in [
        PacketPublicationOrigin::Unknown,
        PacketPublicationOrigin::NativeCapture,
        PacketPublicationOrigin::GraphOutput,
    ]
    .into_iter()
    .enumerate()
    {
        let value = PacketPublicationSample {
            origin,
            buffer_generation: (1 << 53) + index as u64 + 7,
            flags: u32::MAX - index as u32,
            qpc_before_100ns: None,
            qpc_after_100ns: None,
            ..sample(index as u64 + 1)
        };
        writer.record(value);
        assert_eq!(observer.snapshot().unwrap().events[index].sample, value);
    }
    let snapshot = observer.snapshot().unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    for (index, name) in ["unknown", "native_capture", "graph_output"]
        .into_iter()
        .enumerate()
    {
        let json_sample = &json["events"][index]["sample"];
        assert_eq!(json_sample["origin"], name);
        assert_eq!(
            json_sample["buffer_generation"].as_u64(),
            Some((1 << 53) + index as u64 + 7)
        );
        assert_eq!(
            json_sample["flags"].as_u64(),
            Some(u64::from(u32::MAX - index as u32))
        );
        assert!(json_sample["qpc_before_100ns"].is_null());
        assert!(json_sample["qpc_after_100ns"].is_null());
    }
}

#[test]
fn unknown_atomic_enum_word_is_not_silently_decoded_as_unknown_origin() {
    let (mut writer, observer) = packet_history();
    writer.record(sample(1));
    observer.0.events[0][7].store(999, SeqCst);
    assert!(observer.snapshot().is_none());
}

#[test]
fn invalid_atomic_flags_do_not_truncate_to_a_valid_native_flag_word() {
    let (mut writer, observer) = packet_history();
    writer.record(sample(1));
    observer.0.events[0][9].store(u64::MAX, SeqCst);
    assert!(observer.snapshot().is_none());
}

#[test]
fn explicit_saturation_is_sticky_and_keeps_all_valid_prefix_facts() {
    let (mut writer, observer) = packet_history();
    writer.record(sample(1));
    let mut expected = observer.snapshot().unwrap();
    writer.mark_saturated();
    writer.record(sample(2));
    expected.saturated = true;
    assert_eq!(observer.snapshot().unwrap(), expected);
}

#[test]
fn exhausted_totals_sequence_version_and_clock_counters_preserve_prefix() {
    for case in 0..6 {
        let (mut writer, observer) = packet_history();
        writer.record(sample(1));
        let mut expected = observer.snapshot().unwrap();
        let mut next = sample(2);
        match case {
            0 => observer.0.version.store(u64::MAX - 1, SeqCst),
            1 => writer.total = u64::MAX,
            2 => writer.offered = u64::MAX,
            3 => writer.published = u64::MAX,
            4 => {
                writer.unavailable = u64::MAX;
                next.qpc_after_100ns = None;
            }
            5 => {
                writer.regressed = u64::MAX;
                next.qpc_before_100ns = Some(1);
            }
            _ => unreachable!(),
        }
        writer.record(next);
        writer.record(sample(3));
        expected.saturated = true;
        assert_eq!(observer.snapshot().unwrap(), expected, "case {case}");
    }
}

#[test]
fn maximal_representable_frame_totals_are_exact_and_never_wrapped() {
    let (mut writer, observer) = packet_history();
    writer.record(PacketPublicationSample {
        offered_frames: u64::MAX,
        accepted_frames: u64::MAX,
        ..sample(1)
    });
    let before_overflow = observer.snapshot().unwrap();
    assert_eq!(before_overflow.offered_frames, u64::MAX);
    assert_eq!(before_overflow.published_frames, u64::MAX);
    assert!(!before_overflow.saturated);
    writer.record(sample(2));
    let snapshot = observer.snapshot().unwrap();
    assert!(snapshot.saturated);
    assert_eq!(snapshot.total_events, 1);
    assert_eq!(snapshot.events, before_overflow.events);
    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(json["offered_frames"].as_u64(), Some(u64::MAX));
    assert_eq!(json["published_frames"].as_u64(), Some(u64::MAX));
}

#[test]
fn contended_snapshot_returns_none_without_retrying_or_stale_success() {
    let (mut writer, observer) = packet_history();
    writer.record(sample(1));
    observer.0.version.store(3, SeqCst);
    assert!(observer.snapshot().is_none());
}

#[test]
fn observers_own_copies_and_survive_writer_drop_without_session_mixing() {
    let (mut writer, observer) = packet_history();
    writer.record(sample(1));
    let clone = observer.clone();
    let mut snapshot = observer.snapshot().unwrap();
    snapshot.events[0].sample.accepted_frames = 0;
    drop(writer);
    let (mut replacement, peer) = packet_history();
    replacement.record(sample(1));
    replacement.record(sample(2));
    assert_eq!(clone.snapshot().unwrap().events[0].sample, sample(1));
    assert_eq!(clone.snapshot().unwrap().total_events, 1);
    assert_eq!(peer.snapshot().unwrap().total_events, 2);
}

#[test]
fn concurrent_suffix_never_mixes_totals_frontiers_origin_or_payload() {
    let (mut writer, observer) = packet_history();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let thread = std::thread::spawn(move || {
        start.wait();
        for sequence in 1..=20_000 {
            writer.record(sample(sequence));
        }
    });
    barrier.wait();
    for _ in 0..10_000 {
        if let Some(snapshot) = observer.snapshot() {
            assert!(!snapshot.saturated);
            assert_eq!(snapshot.offered_frames, snapshot.total_events * 12);
            assert_eq!(snapshot.published_frames, snapshot.total_events * 8);
            let count = snapshot.total_events.min(256);
            assert_eq!(snapshot.events.len() as u64, count);
            assert_eq!(snapshot.overwritten_events, snapshot.total_events - count);
            for (index, event) in snapshot.events.iter().enumerate() {
                assert_eq!(
                    event.sequence,
                    snapshot.overwritten_events + index as u64 + 1
                );
                assert_eq!(event.offered_before_frames, (event.sequence - 1) * 12);
                assert_eq!(event.published_before_frames, (event.sequence - 1) * 8);
                assert_eq!(event.sample, sample(event.sequence));
            }
        }
    }
    thread.join().unwrap();
    assert_eq!(observer.snapshot().unwrap().total_events, 20_000);
}

#[test]
fn storage_budget_includes_every_scalar_in_the_fixed_256_slot_ring() {
    assert_eq!(
        packet_history_storage_bytes(),
        size_of::<PublishedHistory>()
    );
    assert_eq!(
        packet_history_storage_bytes(),
        (256 * 10 + 7) * size_of::<AtomicU64>()
    );
}
