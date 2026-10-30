//! Cumulative clock loss survives the bounded debug tail and owner retirement.
#[cfg(any(windows, test))]
use super::ClockTick;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualClockCounters {
    pub rendered_periods: u64,
    pub rendered_frames: u64,
    pub late_wake_bursts: u64,
    pub clamped_periods: u64,
    /// Engine frames permanently omitted by the clock, not native PCM loss.
    pub clamped_frames: u64,
    pub timeline_resets: u64,
    /// Includes debt discarded before applying the recovery cap.
    pub max_backlog_periods: u64,
    pub counters_saturated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualClockSnapshot {
    pub schema_version: u32,
    pub engine_sample_rate: u32,
    pub engine_block_frames: u32,
    pub recovery_cap_periods: u64,
    #[serde(flatten)]
    pub counters: VirtualClockCounters,
}

/// One owner writes fixed atomic scalars; control readers never retry or spin.
/// A retained Arc belongs to that owner even after the registry is replaced.
pub struct VirtualClockProbe {
    engine_sample_rate: u32,
    engine_block_frames: u32,
    recovery_cap_periods: u64,
    sequence: AtomicU64,
    fields: [AtomicU64; 8],
}

impl VirtualClockProbe {
    #[cfg(any(windows, test))]
    pub(crate) fn new(rate: u32, block: u32, cap: u64) -> Self {
        Self {
            engine_sample_rate: rate,
            engine_block_frames: block,
            recovery_cap_periods: cap,
            sequence: AtomicU64::new(0),
            fields: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    #[cfg(windows)]
    pub(crate) fn matches(&self, rate: u32, block: u32, cap: u64) -> bool {
        (
            self.engine_sample_rate,
            self.engine_block_frames,
            self.recovery_cap_periods,
        ) == (rate, block, cap)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn publish(&self, counters: VirtualClockCounters) {
        // Single-writer SeqCst publication, matching boundary telemetry. Atomic
        // payloads avoid data races; version exhaustion stays unknown, not ABA.
        let version = self.sequence.load(SeqCst);
        let Some(next) = version.checked_add(2).filter(|_| version.is_multiple_of(2)) else {
            self.sequence.store(u64::MAX, SeqCst);
            return;
        };
        self.sequence.store(version + 1, SeqCst);
        for (slot, value) in self.fields.iter().zip([
            counters.rendered_periods,
            counters.rendered_frames,
            counters.late_wake_bursts,
            counters.clamped_periods,
            counters.clamped_frames,
            counters.timeline_resets,
            counters.max_backlog_periods,
            u64::from(counters.counters_saturated),
        ]) {
            slot.store(value, SeqCst);
        }
        self.sequence.store(next, SeqCst);
    }

    /// None before the owner starts, during publication, or after version
    /// exhaustion. Unknown must never be treated as an observed zero loss.
    pub fn snapshot(&self) -> Option<VirtualClockSnapshot> {
        let before = self.sequence.load(SeqCst);
        if before == 0 || !before.is_multiple_of(2) {
            return None;
        }
        let fields: [u64; 8] = std::array::from_fn(|index| self.fields[index].load(SeqCst));
        (before == self.sequence.load(SeqCst)).then_some(VirtualClockSnapshot {
            schema_version: 1,
            engine_sample_rate: self.engine_sample_rate,
            engine_block_frames: self.engine_block_frames,
            recovery_cap_periods: self.recovery_cap_periods,
            counters: VirtualClockCounters {
                rendered_periods: fields[0],
                rendered_frames: fields[1],
                late_wake_bursts: fields[2],
                clamped_periods: fields[3],
                clamped_frames: fields[4],
                timeline_resets: fields[5],
                max_backlog_periods: fields[6],
                counters_saturated: fields[7] != 0,
            },
        })
    }
}

#[cfg(any(windows, test))]
impl VirtualClockCounters {
    fn add(counter: &mut u64, value: u64, saturated: &mut bool) {
        *counter = counter.checked_add(value).unwrap_or_else(|| {
            *saturated = true;
            u64::MAX
        });
    }

    pub(crate) fn tick(&mut self, tick: ClockTick, new_late_burst: bool, reset: bool) {
        let backlog = tick
            .backlog_periods
            .checked_add(tick.clamped_periods)
            .unwrap_or_else(|| {
                self.counters_saturated = true;
                u64::MAX
            });
        self.max_backlog_periods = self.max_backlog_periods.max(backlog);
        let omitted = tick
            .clamped_periods
            .checked_mul(tick.frames as u64)
            .unwrap_or_else(|| {
                self.counters_saturated = true;
                u64::MAX
            });
        for (counter, value) in [
            (&mut self.clamped_periods, tick.clamped_periods),
            (&mut self.clamped_frames, omitted),
            (&mut self.late_wake_bursts, u64::from(new_late_burst)),
            (&mut self.timeline_resets, u64::from(reset)),
        ] {
            Self::add(counter, value, &mut self.counters_saturated);
        }
    }

    pub(crate) fn rendered(&mut self, frames: usize) {
        Self::add(&mut self.rendered_periods, 1, &mut self.counters_saturated);
        Self::add(
            &mut self.rendered_frames,
            frames as u64,
            &mut self.counters_saturated,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_totals_survive_more_events_than_the_debug_tail() {
        let probe = VirtualClockProbe::new(48_000, 32, 1);
        assert!(probe.snapshot().is_none());
        let mut counters = VirtualClockCounters::default();
        for _ in 0..100 {
            counters.tick(
                ClockTick {
                    frames: 32,
                    backlog_periods: 1,
                    clamped_periods: 2,
                },
                true,
                false,
            );
            counters.rendered(32);
            probe.publish(counters);
        }
        let sample = probe.snapshot().unwrap();
        assert_eq!(sample.counters.clamped_periods, 200);
        assert_eq!(sample.counters.clamped_frames, 6400);
        assert_eq!(sample.counters.rendered_frames, 3200);
        assert_eq!(sample.counters.late_wake_bursts, 100);
        assert_eq!(sample.counters.max_backlog_periods, 3);
    }

    #[test]
    fn contention_and_exhausted_publication_are_unknown_without_retry() {
        let probe = VirtualClockProbe::new(48_000, 32, 1);
        probe.sequence.store(1, SeqCst);
        assert!(probe.snapshot().is_none());
        probe.sequence.store(u64::MAX - 1, SeqCst);
        probe.publish(VirtualClockCounters::default());
        assert!(probe.snapshot().is_none());
        probe.publish(VirtualClockCounters::default());
        assert!(probe.snapshot().is_none());
    }

    #[test]
    fn arithmetic_saturates_and_keeps_a_fault_instead_of_wrapping_clean() {
        let mut counters = VirtualClockCounters {
            clamped_frames: u64::MAX - 1,
            ..Default::default()
        };
        counters.tick(
            ClockTick {
                frames: 32,
                backlog_periods: 1,
                clamped_periods: 1,
            },
            false,
            true,
        );
        assert_eq!(counters.clamped_frames, u64::MAX);
        assert_eq!(counters.timeline_resets, 1);
        assert!(counters.counters_saturated);
    }
}
