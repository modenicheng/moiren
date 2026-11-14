use super::*;

#[test]
fn bounded_history_preserves_retained_order_and_reports_overwrites() {
    let (mut writer, observer) = event_history();
    assert!(observer.snapshot().unwrap().events.is_empty());
    for sequence in 1..=EVENT_HISTORY_CAPACITY as u64 + 3 {
        writer.record(sequence, Some(sequence * 10));
    }
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, EVENT_HISTORY_CAPACITY as u64 + 3);
    assert_eq!(snapshot.overwritten_events, 3);
    assert_eq!(snapshot.events.len(), EVENT_HISTORY_CAPACITY);
    assert_eq!(snapshot.events[0].sequence, 4);
    assert_eq!(
        snapshot.events.last().unwrap().sequence,
        snapshot.total_events
    );
    assert_eq!(snapshot.events[0].frames, 4);
    assert_eq!(snapshot.events[0].qpc_100ns, Some(40));
}

#[test]
fn missing_and_regressed_clocks_are_not_synthesized_or_hidden() {
    let (mut writer, observer) = event_history();
    writer.record(0, Some(1));
    writer.record(32, Some(100));
    writer.record(64, Some(90));
    writer.record(64, None);
    writer.record(64, Some(0));
    writer.record(64, Some(200));
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.total_events, 5);
    assert_eq!(snapshot.unavailable_clocks, 2);
    assert_eq!(snapshot.regressed_clocks, 1);
    assert_eq!(snapshot.events[2].qpc_100ns, None);
    assert!(!snapshot.saturated);
}

#[test]
fn atomic_history_is_coherent_under_concurrent_publication() {
    let (mut writer, observer) = event_history();
    let thread = std::thread::spawn(move || {
        for sequence in 1..=20_000 {
            writer.record(sequence, Some(sequence * 3));
        }
        writer
    });
    for _ in 0..10_000 {
        if let Some(snapshot) = observer.snapshot() {
            assert_eq!(
                snapshot.events.len(),
                (snapshot.total_events as usize).min(EVENT_HISTORY_CAPACITY)
            );
            for event in &snapshot.events {
                assert_eq!(event.frames, event.sequence);
                assert_eq!(event.qpc_100ns, Some(event.sequence * 3));
            }
            if let Some(last) = snapshot.events.last() {
                assert_eq!(last.sequence, snapshot.total_events);
            }
        }
    }
    let _writer = thread.join().unwrap();
    assert_eq!(observer.snapshot().unwrap().total_events, 20_000);
}

#[test]
fn contended_history_returns_unknown_without_retrying() {
    let (_writer, observer) = event_history();
    observer.0.version.store(1, SeqCst);
    assert!(observer.snapshot().is_none());
}

#[test]
fn exhausted_sequence_or_version_invalidates_instead_of_wrapping() {
    for exhaust_version in [false, true] {
        let (mut writer, observer) = event_history();
        writer.record(64, Some(10));
        if exhaust_version {
            observer.0.version.store(u64::MAX - 1, SeqCst);
        } else {
            writer.total = u64::MAX;
        }
        writer.record(128, Some(20));
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.saturated);
        assert_eq!(snapshot.total_events, 1);
        assert_eq!(snapshot.events[0].frames, 64);
    }
}
