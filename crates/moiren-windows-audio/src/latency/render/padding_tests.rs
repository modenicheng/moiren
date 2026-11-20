use super::*;
use std::sync::{Arc, Barrier};

fn sample(value: u32) -> RenderPaddingSample {
    RenderPaddingSample {
        padding_frames: value,
        capacity_frames: value + 100,
        sample_rate: Some(48_000 + value),
        period_frames: None,
    }
}

#[test]
fn interrupted_publication_is_unavailable_then_retry_returns_the_complete_sample() {
    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(1)));
    let reader = observer.clone();
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker_entered = entered.clone();
    let worker_resume = resume.clone();
    let writer = std::thread::spawn(move || {
        let published = owner.publish_with(sample(2), || {
            worker_entered.wait();
            worker_resume.wait();
        });
        (owner, published)
    });
    entered.wait();
    // The real publication is paused after marking its tuple in flight.
    // The reader must neither wait nor fall back to the previous sample.
    let during_publication = reader.try_snapshot();
    resume.wait();
    let (_owner, published) = writer.join().unwrap();
    assert!(published);
    assert_eq!(during_publication, None);
    let after = reader.try_snapshot().unwrap();
    assert_eq!(after.padding_frames, 2);
    assert_eq!(after.capacity_frames, 102);
    assert_eq!(after.sample_rate, Some(48_002));
    assert_eq!(after.sequence, 2);
}

#[test]
fn replacement_during_old_publication_rejects_stale_tuple_and_allows_retry() {
    let observer = RenderPaddingObserver::new();
    let mut old = observer.new_owner();
    assert!(old.publish(sample(1)));
    let mut replacement = None;
    assert!(old.publish_with(sample(2), || {
        let mut new = observer.new_owner();
        assert_eq!(observer.try_snapshot(), None);
        // An old publication is still in flight; the new writer must return
        // rather than spin, and may retry on its next native callback.
        assert!(!new.publish(sample(3)));
        replacement = Some(new);
    }));
    assert_eq!(observer.try_snapshot(), None);
    let mut replacement = replacement.unwrap();
    assert!(replacement.publish(sample(4)));
    old.invalidate();
    drop(old);
    assert_eq!(observer.try_snapshot().unwrap().padding_frames, 4);
}

#[test]
fn reset_during_publication_never_resurrects_the_stopped_owner_sample() {
    let observer = RenderPaddingObserver::new();
    let mut old = observer.new_owner();
    assert!(old.publish(sample(1)));
    assert!(old.publish_with(sample(2), || {
        let mut replacement = observer.new_owner();
        replacement.invalidate();
    }));
    assert_eq!(observer.try_snapshot(), None);
    assert!(!old.publish(sample(3)));
}

#[test]
fn exhausted_sample_sequence_refuses_publication_instead_of_wrapping() {
    let observer = RenderPaddingObserver::new();
    let mut owner = observer.new_owner();
    assert!(owner.publish(sample(1)));
    observer
        .published
        .version
        .store(u64::MAX - 3, std::sync::atomic::Ordering::SeqCst);
    assert!(owner.publish(sample(2)));
    let last = observer.try_snapshot().unwrap();
    assert_eq!(last.sequence, u64::MAX / 2);
    assert_eq!(last.padding_frames, 2);
    assert!(!owner.publish(sample(3)));
    assert_eq!(observer.try_snapshot(), Some(last));
    owner.invalidate();
    assert_eq!(observer.try_snapshot(), None);
}

#[test]
fn exhausted_owner_identity_never_reactivates_a_retired_capability() {
    let observer = RenderPaddingObserver::new();
    let mut old = observer.new_owner();
    old.invalidate();
    observer
        .published
        .next_owner
        .store(u64::MAX - 1, std::sync::atomic::Ordering::SeqCst);
    let mut last = observer.new_owner();
    assert!(last.publish(sample(2)));
    let exhausted = std::panic::catch_unwind(|| observer.new_owner());
    assert!(exhausted.is_err());
    assert!(!old.publish(sample(3)));
    assert_eq!(observer.try_snapshot().unwrap().padding_frames, 2);
}
