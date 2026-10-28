use super::*;

fn boundary(removed: u64, ring: u64, held: u64) -> QueueBoundary {
    QueueBoundary {
        removed_frames: removed,
        ring_frames: ring,
        held_frames: held,
    }
}

fn sample(n: u64) -> QueueReadSample {
    QueueReadSample {
        requested_frames: 8,
        output_frames: 8,
        before: boundary((n - 1) * 8, 32, 0),
        after: boundary(n * 8, 24, 0),
        prime_discarded_frames: 0,
        backlog_discarded_frames: 0,
        age_discarded_frames: 0,
        reset_discarded_frames: 0,
        resets: 0,
        underrun_frames: 0,
        qpc_before_100ns: Some(n * 10),
    }
}

#[test]
fn boundary_formulas_are_checked_in_native_input_frames() {
    let value = boundary(10, 7, 2);
    assert_eq!(value.published_frames(), Some(17));
    assert_eq!(value.retired_frames(), Some(8));
    assert_eq!(value.pending_frames(), Some(9));
    assert_eq!(boundary(u64::MAX, 1, 0).published_frames(), None);
    assert_eq!(boundary(1, 0, 2).retired_frames(), None);
    assert_eq!(boundary(u64::MAX, u64::MAX, 1).pending_frames(), None);
}

#[test]
fn nonzero_requests_keep_zero_output_and_zero_removal_observations() {
    let (mut writer, observer) = queue_history();
    assert!(observer.snapshot().unwrap().events.is_empty());
    let mut value = sample(1);
    value.output_frames = 0;
    value.after = value.before;
    writer.record(value);
    writer.record(QueueReadSample {
        requested_frames: 0,
        ..value
    });
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 1);
    assert_eq!(snapshot.events[0].sample, value);
    assert!(!snapshot.saturated);
}

#[test]
fn bounded_history_keeps_exact_suffix_and_overwrite_counts() {
    let (mut writer, observer) = queue_history();
    for n in 1..=259 {
        writer.record(sample(n));
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 259);
    assert_eq!(snapshot.overwritten_events, 3);
    assert_eq!(snapshot.events.len(), 256);
    assert_eq!(snapshot.events[0].sequence, 4);
    assert_eq!(snapshot.events.last().unwrap().sequence, 259);
    for event in snapshot.events {
        assert_eq!(event.sample, sample(event.sequence));
    }
}

#[test]
fn missing_and_regressed_clocks_remain_diagnostic_and_explicit() {
    let (mut writer, observer) = queue_history();
    let stamps = [Some(100), None, Some(90), Some(0), Some(80), Some(110)];
    for (index, stamp) in stamps.into_iter().enumerate() {
        let mut value = sample(index as u64 + 1);
        value.qpc_before_100ns = stamp;
        writer.record(value);
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 6);
    assert_eq!(snapshot.unavailable_clocks, 2);
    assert_eq!(snapshot.regressed_clocks, 2);
    assert_eq!(snapshot.events[1].sample.qpc_before_100ns, None);
    assert_eq!(snapshot.events[3].sample.qpc_before_100ns, None);
    assert!(!snapshot.saturated);
}

#[test]
fn interpolation_holds_and_all_discard_categories_conserve_inventory() {
    let (mut writer, observer) = queue_history();
    let first = QueueReadSample {
        requested_frames: 1,
        output_frames: 1,
        before: boundary(0, 12, 0),
        after: boundary(4, 8, 2),
        ..sample(1)
    };
    writer.record(first);
    // One output frame need not remove one input frame, and a producer can
    // commit more input between or during these consumer-owned observations.
    writer.record(QueueReadSample {
        before: boundary(4, 16, 2),
        after: boundary(4, 20, 2),
        ..first
    });
    writer.record(QueueReadSample {
        requested_frames: 16,
        output_frames: 0,
        before: boundary(4, 20, 2),
        after: boundary(12, 12, 0),
        prime_discarded_frames: 1,
        backlog_discarded_frames: 2,
        age_discarded_frames: 3,
        reset_discarded_frames: 4,
        resets: 1,
        underrun_frames: 16,
        ..sample(3)
    });
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 3);
    assert!(!snapshot.saturated);
}

#[test]
fn invalid_arithmetic_and_nonconserving_reads_never_publish_an_event() {
    let base = sample(1);
    let invalid = [
        QueueReadSample {
            before: boundary(u64::MAX, 1, 0),
            ..base
        },
        QueueReadSample {
            before: boundary(1, 0, 2),
            ..base
        },
        QueueReadSample {
            before: boundary(u64::MAX, u64::MAX, 1),
            ..base
        },
        QueueReadSample {
            after: boundary(u64::MAX, 1, 0),
            ..base
        },
        QueueReadSample {
            after: boundary(1, 0, 2),
            ..base
        },
        QueueReadSample {
            before: boundary(10, 32, 2),
            after: boundary(9, 33, 2),
            ..base
        },
        QueueReadSample {
            before: boundary(10, 32, 2),
            after: boundary(10, 31, 2),
            ..base
        },
        QueueReadSample {
            before: boundary(10, 32, 0),
            after: boundary(11, 31, 2),
            ..base
        },
        QueueReadSample {
            output_frames: 9,
            ..base
        },
        QueueReadSample {
            underrun_frames: 1,
            ..base
        },
        QueueReadSample {
            prime_discarded_frames: 9,
            ..base
        },
        QueueReadSample {
            prime_discarded_frames: u64::MAX,
            backlog_discarded_frames: 1,
            ..base
        },
        QueueReadSample {
            reset_discarded_frames: 1,
            resets: 0,
            ..base
        },
    ];
    for value in invalid {
        let (mut writer, observer) = queue_history();
        writer.record(value);
        writer.record(base);
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.saturated, "{value:?}");
        assert_eq!(snapshot.total_events, 0, "{value:?}");
        assert!(snapshot.events.is_empty());
    }
}

#[test]
fn consecutive_read_boundaries_cannot_lose_removed_or_held_frames() {
    for next in [
        QueueReadSample {
            before: boundary(7, 32, 0),
            ..sample(2)
        },
        QueueReadSample {
            before: boundary(8, 32, 1),
            ..sample(2)
        },
        QueueReadSample {
            before: boundary(8, 23, 0),
            ..sample(2)
        },
    ] {
        let (mut writer, observer) = queue_history();
        writer.record(sample(1));
        writer.record(next);
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.saturated);
        assert_eq!(snapshot.total_events, 1);
        assert_eq!(snapshot.events[0].sample, sample(1));
    }
}

#[test]
fn maximal_valid_frontiers_do_not_overflow_conservation_intermediates() {
    let (mut writer, observer) = queue_history();
    writer.record(QueueReadSample {
        output_frames: 0,
        before: boundary(0, u64::MAX, 0),
        after: boundary(u64::MAX, 0, 0),
        prime_discarded_frames: u64::MAX,
        ..sample(1)
    });
    assert!(!observer.snapshot().unwrap().saturated);
}

#[test]
fn explicit_saturation_keeps_the_last_valid_history_and_is_sticky() {
    let (mut writer, observer) = queue_history();
    writer.record(sample(1));
    writer.mark_saturated();
    writer.record(sample(2));
    let snapshot = observer.snapshot().unwrap();
    assert!(snapshot.saturated);
    assert_eq!(snapshot.total_events, 1);
    assert_eq!(snapshot.events[0].sample, sample(1));
}

#[test]
fn exhausted_version_sequence_and_clock_counters_do_not_wrap() {
    for case in 0..4 {
        let (mut writer, observer) = queue_history();
        writer.record(sample(1));
        let mut next = sample(2);
        match case {
            0 => observer.0.version.store(u64::MAX - 1, SeqCst),
            1 => writer.total = u64::MAX,
            2 => {
                writer.unavailable = u64::MAX;
                next.qpc_before_100ns = None;
            }
            3 => {
                writer.regressed = u64::MAX;
                next.qpc_before_100ns = Some(1);
            }
            _ => unreachable!(),
        }
        writer.record(next);
        writer.record(sample(3));
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.saturated);
        assert_eq!(snapshot.total_events, 1);
        assert_eq!(snapshot.events[0].sample, sample(1));
    }
}

#[test]
fn contended_history_returns_unknown_without_retrying() {
    let (_writer, observer) = queue_history();
    observer.0.version.store(1, SeqCst);
    assert!(observer.snapshot().is_none());
}

#[test]
fn observers_survive_writer_drop_without_aliasing_or_new_history_mixing() {
    let (mut writer, observer) = queue_history();
    writer.record(sample(1));
    let peer = observer.clone();
    let mut detached = observer.snapshot().unwrap();
    detached.events[0].sample.output_frames = 0;
    drop(writer);
    let (mut replacement, replacement_observer) = queue_history();
    replacement.record(sample(1));
    replacement.record(sample(2));
    assert_eq!(peer.snapshot().unwrap().events[0].sample, sample(1));
    assert_eq!(peer.snapshot().unwrap().total_events, 1);
    assert_eq!(replacement_observer.snapshot().unwrap().total_events, 2);
}

#[test]
fn concurrent_atomic_history_never_mixes_event_fields_or_counts() {
    let (mut writer, observer) = queue_history();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let thread = std::thread::spawn(move || {
        start.wait();
        for n in 1..=20_000 {
            writer.record(sample(n));
        }
    });
    barrier.wait();
    for _ in 0..10_000 {
        if let Some(snapshot) = observer.snapshot() {
            assert!(!snapshot.saturated);
            let count = snapshot.total_events.min(256);
            assert_eq!(snapshot.events.len() as u64, count);
            assert_eq!(snapshot.overwritten_events, snapshot.total_events - count);
            for (index, event) in snapshot.events.iter().enumerate() {
                assert_eq!(
                    event.sequence,
                    snapshot.overwritten_events + index as u64 + 1
                );
                assert_eq!(event.sample, sample(event.sequence));
            }
        }
    }
    thread.join().unwrap();
    assert_eq!(observer.snapshot().unwrap().total_events, 20_000);
}

#[test]
fn storage_budget_includes_all_published_atomic_slots() {
    assert_eq!(queue_history_storage_bytes(), size_of::<PublishedHistory>());
    assert!(queue_history_storage_bytes() >= 256 * 16 * size_of::<AtomicU64>());
}

#[test]
fn snapshots_serialize_diagnostic_unknown_clock_without_synthesizing_time() {
    let (mut writer, observer) = queue_history();
    let mut value = sample(1);
    value.qpc_before_100ns = None;
    writer.record(value);
    let json = serde_json::to_value(observer.snapshot().unwrap()).unwrap();
    assert!(json["events"][0]["sample"]["qpc_before_100ns"].is_null());
    assert_eq!(json["unavailable_clocks"], 1);
    assert_eq!(json["events"][0]["sequence"], 1);
}
