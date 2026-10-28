//! Consumer-owned queue inventory, not source age or alternate-target replay.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::SeqCst},
};

const HISTORY_CAPACITY: usize = 256;
const EVENT_FIELDS: usize = 17;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueBoundary {
    /// Cumulative successful ring pops and committed ring discards.
    pub removed_frames: u64,
    /// Occupancy acquired from the ring by its sole consumer.
    pub ring_frames: u64,
    /// Frames retained outside the ring for interpolation.
    pub held_frames: u64,
}

impl QueueBoundary {
    /// Publication frontier observed through this consumer's acquired tail.
    pub fn published_frames(self) -> Option<u64> {
        self.removed_frames.checked_add(self.ring_frames)
    }

    /// Input frames no longer resident in either ring or interpolation state.
    pub fn retired_frames(self) -> Option<u64> {
        self.removed_frames.checked_sub(self.held_frames)
    }

    pub fn pending_frames(self) -> Option<u64> {
        self.ring_frames.checked_add(self.held_frames)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueReadSample {
    /// Requested/output frames use the consumer rate, not the input rate.
    pub requested_frames: u64,
    pub output_frames: u64,
    pub before: QueueBoundary,
    pub after: QueueBoundary,
    /// Per-read discards use native input frames and must not overlap.
    pub prime_discarded_frames: u64,
    pub backlog_discarded_frames: u64,
    pub age_discarded_frames: u64,
    /// Held-frame losses not already attributed to backlog or age recovery.
    pub reset_discarded_frames: u64,
    /// Per-read reset count, not a cumulative telemetry counter.
    pub resets: u64,
    pub underrun_frames: u64,
    /// Host clock immediately before acquiring the before inventory boundary.
    /// Together with after, brackets the read; not native sample/device time.
    pub qpc_before_100ns: Option<u64>,
    /// Host clock immediately after acquiring the after inventory boundary.
    /// A missing bracket endpoint does not invalidate inventory observations.
    pub qpc_after_100ns: Option<u64>,
}

impl QueueReadSample {
    fn valid(self, previous: Option<QueueBoundary>) -> Option<()> {
        let before_published = self.before.published_frames()?;
        let after_published = self.after.published_frames()?;
        let before_retired = self.before.retired_frames()?;
        let after_retired = self.after.retired_frames()?;
        self.before.pending_frames()?;
        self.after.pending_frames()?;
        self.after
            .removed_frames
            .checked_sub(self.before.removed_frames)?;
        after_published.checked_sub(before_published)?;
        let retired = after_retired.checked_sub(before_retired)?;
        let discarded = self
            .prime_discarded_frames
            .checked_add(self.backlog_discarded_frames)?
            .checked_add(self.age_discarded_frames)?
            .checked_add(self.reset_discarded_frames)?;
        // Held interpolation frames are already removed from the ring. Only
        // retired input, not output-rate demand, can bound native discards.
        retired.checked_sub(discarded)?;
        self.requested_frames
            .checked_sub(self.output_frames.checked_add(self.underrun_frames)?)?;
        if self.reset_discarded_frames > 0 && self.resets == 0 {
            return None;
        }
        if let Some(previous) = previous {
            // Between reads only the producer can move the queue. The next
            // boundary may see more arrivals, but cannot hide consumer losses.
            if self.before.removed_frames != previous.removed_frames
                || self.before.held_frames != previous.held_frames
            {
                return None;
            }
            before_published.checked_sub(previous.published_frames()?)?;
        }
        Some(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueReadEvent {
    pub sequence: u64,
    pub sample: QueueReadSample,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueHistorySnapshot {
    #[serde(deserialize_with = "super::bounded_history::deserialize_events")]
    pub events: Vec<QueueReadEvent>,
    pub total_events: u64,
    pub overwritten_events: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
    pub saturated: bool,
    /// The unique history writer was dropped, not full transport cleanup.
    pub writer_finished: bool,
}

struct PublishedHistory {
    version: AtomicU64,
    total: AtomicU64,
    unavailable: AtomicU64,
    regressed: AtomicU64,
    saturated: AtomicU64,
    finished: AtomicU64,
    events: [[AtomicU64; EVENT_FIELDS]; HISTORY_CAPACITY],
}

impl Default for PublishedHistory {
    fn default() -> Self {
        Self {
            version: AtomicU64::new(0),
            total: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            regressed: AtomicU64::new(0),
            saturated: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            events: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
        }
    }
}

/// Published scalar history storage, excluding Arc allocator bookkeeping.
pub fn queue_history_storage_bytes() -> usize {
    size_of::<PublishedHistory>()
}

/// Allocate off RT. Keep endpoint destruction off RT as with the audio ring.
/// Only the unique writer can publish; observers never retain mutable slots.
pub fn queue_history() -> (QueueHistoryWriter, QueueHistoryObserver) {
    let shared = Arc::new(PublishedHistory::default());
    (
        QueueHistoryWriter {
            shared: shared.clone(),
            total: 0,
            unavailable: 0,
            regressed: 0,
            last_clock: None,
            last_boundary: None,
        },
        QueueHistoryObserver(shared),
    )
}

pub struct QueueHistoryWriter {
    shared: Arc<PublishedHistory>,
    total: u64,
    unavailable: u64,
    regressed: u64,
    last_clock: Option<u64>,
    last_boundary: Option<QueueBoundary>,
}

impl QueueHistoryWriter {
    /// Bounded scalar publication, without allocation, locks or retries.
    /// Zero-output reads still matter; only zero-request calls are omitted.
    /// Invalid arithmetic/conservation preserves the valid prefix and makes
    /// the diagnostic permanently saturated rather than publishing false data.
    pub fn record(&mut self, mut sample: QueueReadSample) {
        if sample.requested_frames == 0 || self.shared.saturated.load(SeqCst) != 0 {
            return;
        }
        let version = self.shared.version.load(SeqCst);
        let (Some(after), Some(total)) = (version.checked_add(2), self.total.checked_add(1)) else {
            self.mark_saturated();
            return;
        };
        if sample.valid(self.last_boundary).is_none() {
            self.mark_saturated();
            return;
        }
        sample.qpc_before_100ns = sample.qpc_before_100ns.filter(|&stamp| stamp != 0);
        sample.qpc_after_100ns = sample.qpc_after_100ns.filter(|&stamp| stamp != 0);
        let regressed = sample
            .qpc_before_100ns
            .zip(self.last_clock)
            .is_some_and(|(stamp, last)| stamp < last)
            || sample
                .qpc_after_100ns
                .zip(sample.qpc_before_100ns)
                .is_some_and(|(after, before)| after < before)
            || sample
                .qpc_after_100ns
                .zip(self.last_clock)
                .is_some_and(|(stamp, last)| stamp < last);
        let missing = sample.qpc_before_100ns.is_none() || sample.qpc_after_100ns.is_none();
        let (Some(unavailable), Some(regressed_count)) = (
            self.unavailable.checked_add(u64::from(missing)),
            self.regressed.checked_add(u64::from(regressed)),
        ) else {
            self.mark_saturated();
            return;
        };
        // Atomic payloads avoid seqlock data races. Exactly one writer owns
        // this version; SeqCst orders both markers and every field for readers.
        self.shared.version.store(version + 1, SeqCst);
        let fields = [
            total,
            sample.requested_frames,
            sample.output_frames,
            sample.before.removed_frames,
            sample.before.ring_frames,
            sample.before.held_frames,
            sample.after.removed_frames,
            sample.after.ring_frames,
            sample.after.held_frames,
            sample.prime_discarded_frames,
            sample.backlog_discarded_frames,
            sample.age_discarded_frames,
            sample.reset_discarded_frames,
            sample.resets,
            sample.underrun_frames,
            sample.qpc_before_100ns.unwrap_or(0),
            sample.qpc_after_100ns.unwrap_or(0),
        ];
        let event = &self.shared.events[((total - 1) % HISTORY_CAPACITY as u64) as usize];
        for (slot, field) in event.iter().zip(fields) {
            slot.store(field, SeqCst);
        }
        self.shared.total.store(total, SeqCst);
        self.shared.unavailable.store(unavailable, SeqCst);
        self.shared.regressed.store(regressed_count, SeqCst);
        self.shared.version.store(after, SeqCst);
        self.total = total;
        self.unavailable = unavailable;
        self.regressed = regressed_count;
        // Missing clocks cannot conceal a regression against the last known
        // stamp; inventory remains useful even when timing is unavailable.
        self.last_clock = sample
            .qpc_after_100ns
            .or(sample.qpc_before_100ns)
            .or(self.last_clock);
        self.last_boundary = Some(sample.after);
    }

    /// Source-local checked counters can invalidate evidence without panicking.
    pub fn mark_saturated(&mut self) {
        self.shared.saturated.store(1, SeqCst);
    }
}

impl Drop for QueueHistoryWriter {
    fn drop(&mut self) {
        // The unique writer cannot publish again. An observer-held Arc keeps
        // storage alive; this marker does not claim full transport cleanup.
        self.shared.finished.store(1, SeqCst);
    }
}

#[derive(Clone)]
pub struct QueueHistoryObserver(Arc<PublishedHistory>);
impl QueueHistoryObserver {
    /// Control-side owned copy. Contention returns None in one attempt, never
    /// a mutable alias, an incoherent event or a stale successful fallback.
    pub fn snapshot(&self) -> Option<QueueHistorySnapshot> {
        let shared = &self.0;
        let before = shared.version.load(SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        // Read closure before every payload field, including saturation which
        // does not advance version. A true marker therefore includes all final
        // writer facts, while the version double-check still rejects races.
        let writer_finished = shared.finished.load(SeqCst) != 0;
        let total = shared.total.load(SeqCst);
        let count = total.min(HISTORY_CAPACITY as u64);
        let mut events = Vec::with_capacity(count as usize);
        for sequence in total - count + 1..=total {
            let slots = &shared.events[((sequence - 1) % HISTORY_CAPACITY as u64) as usize];
            let f: [u64; EVENT_FIELDS] = std::array::from_fn(|index| slots[index].load(SeqCst));
            events.push(QueueReadEvent {
                sequence: f[0],
                sample: QueueReadSample {
                    requested_frames: f[1],
                    output_frames: f[2],
                    before: QueueBoundary {
                        removed_frames: f[3],
                        ring_frames: f[4],
                        held_frames: f[5],
                    },
                    after: QueueBoundary {
                        removed_frames: f[6],
                        ring_frames: f[7],
                        held_frames: f[8],
                    },
                    prime_discarded_frames: f[9],
                    backlog_discarded_frames: f[10],
                    age_discarded_frames: f[11],
                    reset_discarded_frames: f[12],
                    resets: f[13],
                    underrun_frames: f[14],
                    qpc_before_100ns: (f[15] != 0).then_some(f[15]),
                    qpc_after_100ns: (f[16] != 0).then_some(f[16]),
                },
            });
        }
        let snapshot = QueueHistorySnapshot {
            events,
            total_events: total,
            overwritten_events: total - count,
            unavailable_clocks: shared.unavailable.load(SeqCst),
            regressed_clocks: shared.regressed.load(SeqCst),
            saturated: shared.saturated.load(SeqCst) != 0,
            writer_finished,
        };
        (before == shared.version.load(SeqCst)).then_some(snapshot)
    }
}

#[cfg(test)]
mod tests;
