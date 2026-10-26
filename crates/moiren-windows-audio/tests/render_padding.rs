use moiren_windows_audio::render::{RenderPaddingObserver, RenderPaddingSample};
use std::sync::{Arc, Barrier};

mod common;

fn sample(value: u32) -> RenderPaddingSample {
    RenderPaddingSample {
        padding_frames: value,
        capacity_frames: value + 100,
        sample_rate: Some(48_000 + value),
        period_frames: Some(value + 10),
    }
}

#[test]
fn observer_is_unknown_until_a_complete_sample_is_published() {
    let observer = RenderPaddingObserver::new();
    assert_eq!(observer.try_snapshot(), None);

    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(20)));
    let snapshot = observer.try_snapshot().unwrap();
    assert_eq!(snapshot.padding_frames, 20);
    assert_eq!(snapshot.capacity_frames, 120);
    assert_eq!(snapshot.sample_rate, Some(48_020));
    assert_eq!(snapshot.period_frames, Some(30));
    assert_eq!(snapshot.sequence, 1);
}

#[test]
fn serialized_samples_preserve_unknown_metadata_and_snapshot_sequence() {
    let observation = RenderPaddingSample {
        padding_frames: 0,
        capacity_frames: 480,
        sample_rate: Some(48_000),
        period_frames: None,
    };
    let serialized = serde_json::to_value(observation).unwrap();
    assert_eq!(serialized["padding_frames"], 0);
    assert_eq!(serialized["capacity_frames"], 480);
    assert_eq!(serialized["sample_rate"], 48_000);
    assert_eq!(serialized["period_frames"], serde_json::Value::Null);

    let observer = RenderPaddingObserver::new();
    assert_eq!(
        serde_json::to_value(observer.try_snapshot()).unwrap(),
        serde_json::Value::Null
    );
    let mut owner = observer.new_owner();
    assert!(owner.publish(observation));
    let snapshot = serde_json::to_value(observer.try_snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["padding_frames"], serialized["padding_frames"]);
    assert_eq!(snapshot["capacity_frames"], serialized["capacity_frames"]);
    assert_eq!(snapshot["sample_rate"], serialized["sample_rate"]);
    assert_eq!(snapshot["period_frames"], serde_json::Value::Null);
    assert_eq!(snapshot["sequence"], 1);
}

#[test]
fn snapshot_is_coherent_while_the_owner_replaces_values() {
    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(0)));
    let reader = observer.clone();
    let start = Arc::new(Barrier::new(2));
    let writer_start = start.clone();
    let writer = std::thread::spawn(move || {
        writer_start.wait();
        for value in 1..50_000 {
            assert!(owner.publish(sample(value)));
        }
        owner
    });

    start.wait();
    let mut last_sequence = 0;
    for _ in 0..100_000 {
        if let Some(snapshot) = reader.try_snapshot() {
            assert_eq!(snapshot.capacity_frames, snapshot.padding_frames + 100);
            assert_eq!(snapshot.sample_rate, Some(48_000 + snapshot.padding_frames));
            assert_eq!(snapshot.period_frames, Some(snapshot.padding_frames + 10));
            assert_eq!(snapshot.sequence, u64::from(snapshot.padding_frames) + 1);
            assert!(snapshot.sequence >= last_sequence);
            last_sequence = snapshot.sequence;
        }
    }
    let owner = writer.join().unwrap();
    let final_sample = observer.try_snapshot().unwrap();
    assert_eq!(final_sample.padding_frames, 49_999);
    assert_eq!(final_sample.sequence, 50_000);
    drop(owner);
}

#[test]
fn invalidation_removes_the_last_padding_sample() {
    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(20)));
    assert!(observer.try_snapshot().is_some());

    owner.invalidate();

    assert_eq!(observer.try_snapshot(), None);
    assert!(!owner.publish(sample(30)));
}

#[test]
fn replacement_owner_isolated_from_the_previous_owner() {
    let observer = RenderPaddingObserver::new();
    let mut first = observer.new_owner();
    assert!(first.publish(sample(20)));
    let first_sequence = observer.try_snapshot().unwrap().sequence;

    let mut second = observer.new_owner();
    assert_eq!(observer.try_snapshot(), None);
    assert!(!first.publish(sample(30)));
    assert!(second.publish(sample(40)));
    assert_eq!(observer.try_snapshot().unwrap().padding_frames, 40);
    assert!(!first.publish(sample(50)));
    first.invalidate();
    drop(first);
    let snapshot = observer.try_snapshot().unwrap();
    assert_eq!(snapshot.padding_frames, 40);
    assert!(snapshot.sequence > first_sequence);
}

#[test]
fn dropping_the_owner_invalidates_all_observer_clones() {
    let observer = RenderPaddingObserver::new();
    let reader = observer.clone();
    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(0)));
    assert_eq!(reader.try_snapshot().unwrap().padding_frames, 0);
    drop(owner);
    assert_eq!(observer.try_snapshot(), None);
    assert_eq!(reader.try_snapshot(), None);
}

#[test]
fn missing_metadata_replaces_previous_values_without_treating_zero_as_unknown() {
    let observer = RenderPaddingObserver::default();
    let mut owner = observer.new_owner();
    for (index, metadata) in [Some(48_000), None, Some(0), Some(u32::MAX)]
        .into_iter()
        .enumerate()
    {
        assert!(owner.publish(RenderPaddingSample {
            padding_frames: 0,
            capacity_frames: u32::MAX,
            sample_rate: metadata,
            period_frames: metadata,
        }));
        let snapshot = observer.try_snapshot().unwrap();
        assert_eq!(snapshot.padding_frames, 0);
        assert_eq!(snapshot.capacity_frames, u32::MAX);
        assert_eq!(snapshot.sample_rate, metadata);
        assert_eq!(snapshot.period_frames, metadata);
        assert_eq!(snapshot.sequence, index as u64 + 1);
    }
}

#[test]
fn owner_thread_publication_and_invalidation_do_not_allocate_or_free() {
    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    let mut stale = observer.new_owner();
    let mut current = observer.new_owner();
    let worker = std::thread::spawn(move || {
        common::assert_allocates_nothing(|| {
            assert!(!owner.publish(sample(1)));
            for value in 1..10_000 {
                assert!(current.publish(sample(value)));
                assert!(observer.try_snapshot().is_some());
            }
            stale.invalidate();
            assert!(current.publish(sample(0)));
            current.invalidate();
            assert!(!current.publish(sample(1)));
            assert_eq!(observer.try_snapshot(), None);
        });
        // Return every handle so the joining control thread reclaims Arc storage.
        (observer, owner, stale, current)
    });
    drop(worker.join().unwrap());
}

#[test]
fn live_padding_does_not_change_existing_empty_padding_diagnostics() {
    use moiren_windows_audio::{
        latency::TelemetryMode,
        render::{RenderCallback, RenderDiagnostics, RenderStats},
    };

    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    let stats = RenderStats::default();
    let mut diagnostics = RenderDiagnostics::new(TelemetryMode::Trace, 75_000);
    for value in [20, 0, 10] {
        assert!(owner.publish(sample(value)));
        diagnostics.record(RenderCallback {
            padding_before_frames: value,
            ..Default::default()
        });
    }
    assert_eq!(stats.empty_padding_wakes, 0);
    let events = diagnostics.finish().events;
    assert_eq!(events.len(), 1);
    assert!(events[0].empty_padding);
    assert!(!events[0].bridge_underrun);
}

#[test]
fn replacement_owner_remains_current_when_stale_owner_stops_on_another_thread() {
    let observer = RenderPaddingObserver::new();
    let mut first = observer.new_owner();
    assert!(first.publish(sample(20)));
    let mut second = observer.new_owner();
    assert!(second.publish(sample(40)));
    let stale_worker = std::thread::spawn(move || {
        for value in 1..128 {
            assert!(!first.publish(sample(value)));
        }
        first.invalidate();
        first
    });
    for value in 128..256 {
        assert!(second.publish(sample(value)));
    }
    drop(stale_worker.join().unwrap());
    assert_eq!(observer.try_snapshot().unwrap().padding_frames, 255);
}
