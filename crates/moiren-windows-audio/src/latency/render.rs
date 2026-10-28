//! Bounded native anomaly windows. Preparation allocates the event storage;
//! callbacks only copy scalar observations, and the joined report owns cleanup.
use crate::{clock_bridge::BridgeSnapshot, latency::TelemetryMode};
use serde::{Serialize, Serializer};
use std::ops::Deref;

mod padding;
pub use padding::{
    RenderPaddingObserver, RenderPaddingOwner, RenderPaddingSample, RenderPaddingSnapshot,
};

pub const RENDER_EVENT_CAPACITY: usize = 128;
const HISTORY_CAPACITY: usize = 8;

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct RenderClockSample {
    /// Raw IAudioClock units; normalize by the report's GetFrequency value,
    /// never by the client sample rate. S_FALSE remains visible as HRESULT 1.
    pub position_units: u64,
    pub qpc_100ns: u64,
    pub hresult: i32,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct RenderCallback {
    pub wake_qpc_100ns: Option<u64>,
    pub wake_interval_100ns: Option<u64>,
    pub padding_before_frames: u32,
    pub demand_frames: u32,
    pub submitted_frames_before: u64,
    /// Sampled immediately after padding, not atomically with that query.
    pub native_clock: Option<RenderClockSample>,
    /// Rendering or native submission failed; demand is not a submitted count.
    pub failed: bool,
    /// Approximate scalar bridge observations, not a cross-owner transaction.
    pub bridge_before: Option<BridgeSnapshot>,
    pub bridge_after: Option<BridgeSnapshot>,
}

#[derive(Debug, Clone, Copy)]
pub struct RenderCallbackHistory {
    records: [RenderCallback; HISTORY_CAPACITY],
    len: usize,
}
impl Deref for RenderCallbackHistory {
    type Target = [RenderCallback];
    fn deref(&self) -> &Self::Target {
        &self.records[..self.len]
    }
}
impl Serialize for RenderCallbackHistory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.deref().serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RenderEvent {
    pub callback: RenderCallback,
    pub previous_callbacks: RenderCallbackHistory,
    /// Wake interval exceeded the admitted render target's duration.
    pub wake_over_budget: bool,
    /// Empty padding is diagnostic evidence, not a hardware XRUN measurement.
    pub empty_padding: bool,
    pub bridge_recovered: bool,
    pub bridge_underrun: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct RenderDiagnosticsSnapshot {
    pub enabled: bool,
    pub wake_budget_100ns: u64,
    pub overwritten_events: u64,
    /// Most recent bounded anomaly windows, ordered by callback arrival.
    pub events: Vec<RenderEvent>,
    /// Public finish transfers prepared storage to the snapshot, so finishing
    /// inside a callback cannot free the boxed history on that thread.
    #[serde(skip)]
    retained_history: Option<Box<RenderCallbackHistory>>,
}

#[derive(Debug)]
pub struct RenderDiagnostics {
    enabled: bool,
    wake_budget_100ns: u64,
    previous_wake: Option<u64>,
    // Live bridge DTOs make a callback history large. Allocate it once before
    // streaming rather than embedding it in every owner/cleanup report copy.
    history: Box<RenderCallbackHistory>,
    events: Vec<RenderEvent>,
    next_event: usize,
    overwritten_events: u64,
    #[cfg(windows)]
    finalized: bool,
}
impl RenderDiagnostics {
    /// Must run before streaming: reserve every event slot once.
    pub fn new(mode: TelemetryMode, wake_budget_100ns: u64) -> Self {
        let enabled = mode == TelemetryMode::Trace;
        Self {
            enabled,
            wake_budget_100ns,
            previous_wake: None,
            history: Box::new(RenderCallbackHistory {
                records: [RenderCallback::default(); HISTORY_CAPACITY],
                len: 0,
            }),
            events: Vec::with_capacity(if enabled { RENDER_EVENT_CAPACITY } else { 0 }),
            next_event: 0,
            overwritten_events: 0,
            #[cfg(windows)]
            finalized: false,
        }
    }
    #[cfg(windows)]
    pub fn set_wake_budget(&mut self, wake_budget_100ns: u64) {
        self.wake_budget_100ns = wake_budget_100ns;
    }
    /// Bounded, allocation-free owner-local copies. No logging or quantiles.
    pub fn record(&mut self, mut callback: RenderCallback) {
        if !self.enabled {
            return;
        }
        callback.wake_interval_100ns = callback
            .wake_qpc_100ns
            .zip(self.previous_wake)
            .and_then(|(current, previous)| current.checked_sub(previous));
        self.previous_wake = callback.wake_qpc_100ns;
        let wake_over_budget = callback
            .wake_interval_100ns
            .is_some_and(|interval| interval > self.wake_budget_100ns);
        let empty_padding = callback.padding_before_frames == 0;
        let bridge_pair = callback.bridge_before.zip(callback.bridge_after);
        let bridge_recovered = bridge_pair.is_some_and(|(before, after)| {
            after.backlog_recoveries > before.backlog_recoveries
                || after.age_recoveries > before.age_recoveries
        });
        let bridge_underrun = bridge_pair
            .is_some_and(|(before, after)| after.underrun_frames > before.underrun_frames);
        if wake_over_budget
            || empty_padding
            || bridge_recovered
            || bridge_underrun
            || callback.failed
        {
            let event = RenderEvent {
                callback,
                previous_callbacks: *self.history,
                wake_over_budget,
                empty_padding,
                bridge_recovered,
                bridge_underrun,
            };
            if self.events.len() < RENDER_EVENT_CAPACITY {
                self.events.push(event);
            } else {
                self.events[self.next_event] = event;
                self.next_event = (self.next_event + 1) % RENDER_EVENT_CAPACITY;
                self.overwritten_events = self.overwritten_events.saturating_add(1);
            }
        }
        if self.history.len == HISTORY_CAPACITY {
            self.history.records.copy_within(1..HISTORY_CAPACITY, 0);
        } else {
            self.history.len += 1;
        }
        self.history.records[self.history.len - 1] = callback;
    }
    /// Move storage to the report after streaming; its control-side owner drops it.
    pub fn finish(mut self) -> RenderDiagnosticsSnapshot {
        let mut snapshot = self.take_snapshot();
        snapshot.retained_history = Some(self.history);
        snapshot
    }
    /// Retain both prepared allocations in the report until the control joiner
    /// drops it, including after a native callback unwinds.
    #[cfg(windows)]
    pub fn drain_snapshot(&mut self) -> Option<RenderDiagnosticsSnapshot> {
        if self.finalized {
            return None;
        }
        self.finalized = true;
        Some(self.take_snapshot())
    }
    fn take_snapshot(&mut self) -> RenderDiagnosticsSnapshot {
        self.events.rotate_left(self.next_event);
        self.next_event = 0;
        RenderDiagnosticsSnapshot {
            enabled: self.enabled,
            wake_budget_100ns: self.wake_budget_100ns,
            overwritten_events: self.overwritten_events,
            events: std::mem::take(&mut self.events),
            retained_history: None,
        }
    }
}
