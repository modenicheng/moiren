use moiren_engine::runtime::Engine;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineSnapshot {
    pub frame: u64,
    pub epoch: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedTimelineSnapshot {
    pub owner_generation: u64,
    pub frame: u64,
    pub epoch: u64,
    pub revision: u64,
}

struct PublishedTimeline {
    sequence: AtomicU64,
    frame: AtomicU64,
    epoch: AtomicU64,
    revision: AtomicU64,
}

#[derive(Clone)]
pub struct TimelineObserver {
    published: Arc<PublishedTimeline>,
    initial: TimelineSnapshot,
    owner_generation: u64,
}

impl TimelineObserver {
    pub(super) fn new(owner_generation: u64, engine: &Engine<f32>) -> Self {
        let initial = TimelineSnapshot {
            frame: engine.timeline(),
            epoch: engine.epoch(),
            revision: engine.revision(),
        };
        Self {
            published: Arc::new(PublishedTimeline {
                sequence: AtomicU64::new(0),
                frame: AtomicU64::new(initial.frame),
                epoch: AtomicU64::new(initial.epoch),
                revision: AtomicU64::new(initial.revision),
            }),
            initial,
            owner_generation,
        }
    }

    pub(super) fn publish(&self, engine: &Engine<f32>) {
        let sequence = self.published.sequence.load(Ordering::SeqCst);
        self.published
            .sequence
            .store(sequence.wrapping_add(1), Ordering::SeqCst);
        self.published
            .frame
            .store(engine.timeline(), Ordering::SeqCst);
        self.published.epoch.store(engine.epoch(), Ordering::SeqCst);
        self.published
            .revision
            .store(engine.revision(), Ordering::SeqCst);
        self.published
            .sequence
            .store(sequence.wrapping_add(2), Ordering::SeqCst);
    }

    /// One bounded attempt. A concurrent publication yields `None`; all scalar
    /// accesses use one total order so an unchanged even sequence proves coherence.
    pub fn try_snapshot(&self) -> Option<TimelineSnapshot> {
        let before = self.published.sequence.load(Ordering::SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        let snapshot = TimelineSnapshot {
            frame: self.published.frame.load(Ordering::SeqCst),
            epoch: self.published.epoch.load(Ordering::SeqCst),
            revision: self.published.revision.load(Ordering::SeqCst),
        };
        (before == self.published.sequence.load(Ordering::SeqCst)).then_some(snapshot)
    }

    /// Legacy callers retain the original shape. If publication is in flight,
    /// the preparation snapshot is an older coherent fallback, never a mixed tuple.
    pub fn snapshot(&self) -> TimelineSnapshot {
        self.try_snapshot().unwrap_or(self.initial)
    }

    pub fn managed_snapshot(&self) -> Option<ManagedTimelineSnapshot> {
        self.try_snapshot().map(|snapshot| ManagedTimelineSnapshot {
            owner_generation: self.owner_generation,
            frame: snapshot.frame,
            epoch: snapshot.epoch,
            revision: snapshot.revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer() -> TimelineObserver {
        TimelineObserver {
            published: Arc::new(PublishedTimeline {
                sequence: AtomicU64::new(0),
                frame: AtomicU64::new(8),
                epoch: AtomicU64::new(1),
                revision: AtomicU64::new(1),
            }),
            initial: TimelineSnapshot {
                frame: 8,
                epoch: 1,
                revision: 1,
            },
            owner_generation: 7,
        }
    }

    #[test]
    fn interrupted_publication_returns_unavailable_or_a_coherent_legacy_fallback() {
        let observer = observer();
        observer.published.sequence.store(1, Ordering::SeqCst);
        observer.published.frame.store(16, Ordering::SeqCst);
        assert_eq!(observer.try_snapshot(), None);
        assert_eq!(observer.managed_snapshot(), None);
        assert_eq!(observer.snapshot(), observer.initial);
    }

    #[test]
    fn concurrent_revision_epoch_and_frame_changes_never_produce_a_mixed_snapshot() {
        let observer = observer();
        let writer = observer.published.clone();
        let worker = std::thread::spawn(move || {
            for revision in 2..10_000 {
                writer.sequence.store(revision * 2 - 1, Ordering::SeqCst);
                writer.frame.store(revision * 8, Ordering::SeqCst);
                writer.epoch.store(revision, Ordering::SeqCst);
                writer.revision.store(revision, Ordering::SeqCst);
                writer.sequence.store(revision * 2, Ordering::SeqCst);
            }
        });
        for _ in 0..20_000 {
            if let Some(snapshot) = observer.managed_snapshot() {
                assert_eq!(snapshot.owner_generation, 7);
                assert_eq!(snapshot.frame, snapshot.revision * 8);
                assert_eq!(snapshot.epoch, snapshot.revision);
            }
        }
        worker.join().unwrap();
        assert_eq!(observer.snapshot().revision, 9_999);
    }
}
