//! Bounded one-writer service-event history; neither sample age nor device time.
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::SeqCst},
};

pub const EVENT_HISTORY_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimedFrameEvent {
    pub sequence: u64,
    pub frames: u64,
    pub qpc_100ns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventHistorySnapshot {
    pub events: Vec<TimedFrameEvent>,
    pub total_events: u64,
    pub overwritten_events: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
    pub saturated: bool,
}

struct PublishedHistory {
    version: AtomicU64,
    total: AtomicU64,
    unavailable: AtomicU64,
    regressed: AtomicU64,
    saturated: AtomicU64,
    events: [[AtomicU64; 3]; EVENT_HISTORY_CAPACITY],
}

impl Default for PublishedHistory {
    fn default() -> Self {
        Self {
            version: AtomicU64::new(0),
            total: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            regressed: AtomicU64::new(0),
            saturated: AtomicU64::new(0),
            events: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
        }
    }
}

/// Allocate off RT. The unique writer is never cloned or placed in a registry;
/// its observation-only peer cannot publish or redirect a generation.
pub fn event_history() -> (EventHistoryWriter, EventHistoryObserver) {
    let shared = Arc::new(PublishedHistory::default());
    (
        EventHistoryWriter {
            shared: shared.clone(),
            total: 0,
            unavailable: 0,
            regressed: 0,
            last: None,
        },
        EventHistoryObserver(shared),
    )
}

pub struct EventHistoryWriter {
    shared: Arc<PublishedHistory>,
    total: u64,
    unavailable: u64,
    regressed: u64,
    last: Option<u64>,
}

impl EventHistoryWriter {
    /// Scalar publication only. Zero-frame calls are not supply/demand events.
    /// Missing clocks remain explicit; exhaustion permanently invalidates data.
    pub fn record(&mut self, frames: u64, qpc_100ns: Option<u64>) {
        if frames == 0 {
            return;
        }
        let shared = &self.shared;
        let before = shared.version.load(SeqCst);
        let (Some(after), Some(total)) = (before.checked_add(2), self.total.checked_add(1)) else {
            shared.saturated.store(1, SeqCst);
            return;
        };
        let qpc = qpc_100ns.filter(|&stamp| stamp != 0);
        if let Some(stamp) = qpc {
            if self.last.is_some_and(|last| stamp < last) {
                self.regressed = self.regressed.saturating_add(1);
            }
        } else {
            self.unavailable = self.unavailable.saturating_add(1);
        }
        self.last = qpc;
        self.total = total;
        shared.version.store(before + 1, SeqCst);
        let event = &shared.events[((total - 1) % EVENT_HISTORY_CAPACITY as u64) as usize];
        event[0].store(total, SeqCst);
        event[1].store(frames, SeqCst);
        event[2].store(qpc.unwrap_or(0), SeqCst);
        shared.total.store(total, SeqCst);
        shared.unavailable.store(self.unavailable, SeqCst);
        shared.regressed.store(self.regressed, SeqCst);
        shared.version.store(after, SeqCst);
    }
}

/// Control-side snapshots allocate a bounded owned copy. A concurrent write
/// returns None without retry; no stale fallback or mutable ring aliasing.
#[derive(Clone)]
pub struct EventHistoryObserver(Arc<PublishedHistory>);

impl EventHistoryObserver {
    pub fn snapshot(&self) -> Option<EventHistorySnapshot> {
        let shared = &self.0;
        let before = shared.version.load(SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        let total = shared.total.load(SeqCst);
        let count = total.min(EVENT_HISTORY_CAPACITY as u64);
        let mut events = Vec::with_capacity(count as usize);
        for sequence in total - count + 1..=total {
            let event = &shared.events[((sequence - 1) % EVENT_HISTORY_CAPACITY as u64) as usize];
            let stamp = event[2].load(SeqCst);
            events.push(TimedFrameEvent {
                sequence: event[0].load(SeqCst),
                frames: event[1].load(SeqCst),
                qpc_100ns: (stamp != 0).then_some(stamp),
            });
        }
        let snapshot = EventHistorySnapshot {
            events,
            total_events: total,
            overwritten_events: total - count,
            unavailable_clocks: shared.unavailable.load(SeqCst),
            regressed_clocks: shared.regressed.load(SeqCst),
            saturated: shared.saturated.load(SeqCst) != 0,
        };
        (before == shared.version.load(SeqCst)).then_some(snapshot)
    }
}

#[cfg(test)]
mod tests;
