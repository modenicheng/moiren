//! Bounded producer-owned packet publication envelopes, not device/sample time.
//!
//! Host QPC brackets describe SPSC publication commits, not WASAPI packet
//! timestamps, source age or end-to-end latency. Origins, generations and
//! native flags remain separate evidence; no continuity is inferred from them.
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::SeqCst},
};

const HISTORY_CAPACITY: usize = 256;
const EVENT_FIELDS: usize = 10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum PacketPublicationOrigin {
    #[default]
    Unknown = 0,
    NativeCapture = 1,
    GraphOutput = 2,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PacketPublicationSample {
    pub offered_frames: u64,
    /// The prefix actually committed to the ring, including zero on full drop.
    pub accepted_frames: u64,
    /// Host publication bracket, never a native sample/device timestamp.
    pub qpc_before_100ns: Option<u64>,
    pub qpc_after_100ns: Option<u64>,
    pub origin: PacketPublicationOrigin,
    /// Local generation of this prefix, before any overflow-driven next epoch.
    pub buffer_generation: u64,
    /// Native discontinuity/timestamp flags, retained without interpretation.
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PacketPublicationEvent {
    pub sequence: u64,
    /// Cumulative frontiers before this event, even in a retained suffix.
    pub offered_before_frames: u64,
    pub published_before_frames: u64,
    pub sample: PacketPublicationSample,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PacketHistorySnapshot {
    pub events: Vec<PacketPublicationEvent>,
    pub total_events: u64,
    pub overwritten_events: u64,
    pub offered_frames: u64,
    pub published_frames: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
    pub saturated: bool,
    /// The unique history writer was dropped, not full transport cleanup.
    pub writer_finished: bool,
}

struct PublishedHistory {
    version: AtomicU64,
    total: AtomicU64,
    offered: AtomicU64,
    published: AtomicU64,
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
            offered: AtomicU64::new(0),
            published: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            regressed: AtomicU64::new(0),
            saturated: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            events: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
        }
    }
}

/// Published scalar storage, excluding Arc allocator bookkeeping.
pub fn packet_history_storage_bytes() -> usize {
    size_of::<PublishedHistory>()
}

/// Allocate and destroy the endpoints off RT, just as with the audio ring.
pub fn packet_history() -> (PacketHistoryWriter, PacketHistoryObserver) {
    let shared = Arc::new(PublishedHistory::default());
    (
        PacketHistoryWriter {
            shared: shared.clone(),
            total: 0,
            offered: 0,
            published: 0,
            unavailable: 0,
            regressed: 0,
            last_clock: None,
        },
        PacketHistoryObserver(shared),
    )
}

pub struct PacketHistoryWriter {
    shared: Arc<PublishedHistory>,
    total: u64,
    offered: u64,
    published: u64,
    unavailable: u64,
    regressed: u64,
    last_clock: Option<u64>,
}

impl PacketHistoryWriter {
    /// Fixed scalar publication with no allocation, locks or retries. Invalid
    /// counts preserve the last valid prefix and permanently saturate evidence.
    pub fn record(&mut self, mut sample: PacketPublicationSample) {
        if self.shared.saturated.load(SeqCst) != 0 {
            return;
        }
        if sample.accepted_frames > sample.offered_frames {
            self.mark_saturated();
            return;
        }
        if sample.offered_frames == 0 {
            return;
        }
        let version = self.shared.version.load(SeqCst);
        let (Some(after), Some(total), Some(offered), Some(published)) = (
            version.checked_add(2),
            self.total.checked_add(1),
            self.offered.checked_add(sample.offered_frames),
            self.published.checked_add(sample.accepted_frames),
        ) else {
            self.mark_saturated();
            return;
        };
        sample.qpc_before_100ns = sample.qpc_before_100ns.filter(|&stamp| stamp != 0);
        sample.qpc_after_100ns = sample.qpc_after_100ns.filter(|&stamp| stamp != 0);
        let is_regressed = sample
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
        let (Some(unavailable), Some(regressed)) = (
            self.unavailable.checked_add(u64::from(missing)),
            self.regressed.checked_add(u64::from(is_regressed)),
        ) else {
            self.mark_saturated();
            return;
        };
        // Every payload field is atomic. The sole writer and SeqCst ordering
        // of both markers and scalars make a matching even-version read coherent.
        self.shared.version.store(version + 1, SeqCst);
        let fields = [
            total,
            self.offered,
            self.published,
            sample.offered_frames,
            sample.accepted_frames,
            sample.qpc_before_100ns.unwrap_or(0),
            sample.qpc_after_100ns.unwrap_or(0),
            sample.origin as u64,
            sample.buffer_generation,
            u64::from(sample.flags),
        ];
        let event = &self.shared.events[((total - 1) % HISTORY_CAPACITY as u64) as usize];
        for (slot, field) in event.iter().zip(fields) {
            slot.store(field, SeqCst);
        }
        self.shared.total.store(total, SeqCst);
        self.shared.offered.store(offered, SeqCst);
        self.shared.published.store(published, SeqCst);
        self.shared.unavailable.store(unavailable, SeqCst);
        self.shared.regressed.store(regressed, SeqCst);
        self.shared.version.store(after, SeqCst);
        self.total = total;
        self.offered = offered;
        self.published = published;
        self.unavailable = unavailable;
        self.regressed = regressed;
        // Missing boundaries cannot erase the last observed clock. Retain
        // regressed stamps verbatim rather than fabricating monotonic time.
        self.last_clock = sample
            .qpc_after_100ns
            .or(sample.qpc_before_100ns)
            .or(self.last_clock);
    }

    /// Mark source-local arithmetic failure without publishing invented facts.
    pub fn mark_saturated(&mut self) {
        self.shared.saturated.store(1, SeqCst);
    }
}

impl Drop for PacketHistoryWriter {
    fn drop(&mut self) {
        // The unique writer cannot publish again. An observer-held Arc keeps
        // storage alive; this marker does not claim full transport cleanup.
        self.shared.finished.store(1, SeqCst);
    }
}

#[derive(Clone)]
pub struct PacketHistoryObserver(Arc<PublishedHistory>);

impl PacketHistoryObserver {
    /// Control-side owned suffix. A single attempt returns None on contention
    /// or invalid scalar encoding; there is no retry or stale-success fallback.
    pub fn snapshot(&self) -> Option<PacketHistorySnapshot> {
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
            // These words only come from typed writer values. Never silently
            // map an invalid enum discriminant to truthful Unknown provenance.
            let origin = match f[7] {
                0 => PacketPublicationOrigin::Unknown,
                1 => PacketPublicationOrigin::NativeCapture,
                2 => PacketPublicationOrigin::GraphOutput,
                _ => return None,
            };
            events.push(PacketPublicationEvent {
                sequence: f[0],
                offered_before_frames: f[1],
                published_before_frames: f[2],
                sample: PacketPublicationSample {
                    offered_frames: f[3],
                    accepted_frames: f[4],
                    qpc_before_100ns: (f[5] != 0).then_some(f[5]),
                    qpc_after_100ns: (f[6] != 0).then_some(f[6]),
                    origin,
                    buffer_generation: f[8],
                    flags: u32::try_from(f[9]).ok()?,
                },
            });
        }
        let snapshot = PacketHistorySnapshot {
            events,
            total_events: total,
            overwritten_events: total - count,
            offered_frames: shared.offered.load(SeqCst),
            published_frames: shared.published.load(SeqCst),
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
