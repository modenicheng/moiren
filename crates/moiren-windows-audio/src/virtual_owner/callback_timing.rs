//! Owner-local Engine service time, never pacing or physical presentation time.
#[cfg(windows)]
use crate::latency::QpcClock;
#[cfg(windows)]
use moiren_engine::{
    runtime::{Engine, RenderReport, RuntimeError},
    sample::ProcessingSample,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualCallbackOutcome {
    Unfinished,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCallbackSample {
    pub attempt_sequence: u64,
    pub epoch_before: u64,
    pub frame_before: u64,
    pub epoch_after: Option<u64>,
    pub frame_after: Option<u64>,
    pub requested_frames: u64,
    pub outcome: VirtualCallbackOutcome,
    pub qpc_before_100ns: Option<u64>,
    pub qpc_after_100ns: Option<u64>,
    pub duration_100ns: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCallbackSnapshot {
    pub attempts: u64,
    pub completed: u64,
    pub errors: u64,
    pub valid_timings: u64,
    /// Completed attempts with at least one absent/zero clock endpoint.
    pub missing_clocks: u64,
    /// Completed attempts with a backward pair or before the prior endpoint.
    pub regressed_clocks: u64,
    pub max_duration_100ns: Option<u64>,
    pub saturated: bool,
    pub latest: Option<VirtualCallbackSample>,
}

const FIELDS: usize = 18;

/// One writer; atomic payloads and a single checked read, never retry/spin.
pub(crate) struct VirtualCallbackProbe {
    #[cfg(windows)]
    clock: Option<QpcClock>,
    sequence: AtomicU64,
    fields: [AtomicU64; FIELDS],
}

impl VirtualCallbackProbe {
    #[cfg(any(windows, test))]
    pub(crate) fn new() -> Self {
        Self {
            // Called only by an explicitly enabled preparation-side builder.
            #[cfg(windows)]
            clock: QpcClock::new(),
            sequence: AtomicU64::new(0),
            fields: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    #[cfg(windows)]
    pub(crate) fn now(&self) -> Option<u64> {
        self.clock.and_then(|clock| clock.now())
    }

    #[cfg(any(windows, test))]
    fn publish(&self, value: VirtualCallbackSnapshot) {
        let version = self.sequence.load(SeqCst);
        let Some(next) = version.checked_add(2).filter(|_| version.is_multiple_of(2)) else {
            self.sequence.store(u64::MAX, SeqCst);
            return;
        };
        let latest = value.latest;
        let mut flags = u64::from(value.saturated);
        flags |= u64::from(value.max_duration_100ns.is_some()) << 1;
        flags |= u64::from(latest.is_some()) << 2;
        if let Some(sample) = latest {
            flags |= u64::from(sample.epoch_after.is_some()) << 3;
            flags |= u64::from(sample.frame_after.is_some()) << 4;
            flags |= u64::from(sample.qpc_before_100ns.is_some()) << 5;
            flags |= u64::from(sample.qpc_after_100ns.is_some()) << 6;
            flags |= u64::from(sample.duration_100ns.is_some()) << 7;
        }
        let fields = [
            value.attempts,
            value.completed,
            value.errors,
            value.valid_timings,
            value.missing_clocks,
            value.regressed_clocks,
            value.max_duration_100ns.unwrap_or(0),
            flags,
            latest.map_or(0, |s| s.attempt_sequence),
            latest.map_or(0, |s| s.epoch_before),
            latest.map_or(0, |s| s.frame_before),
            latest.and_then(|s| s.epoch_after).unwrap_or(0),
            latest.and_then(|s| s.frame_after).unwrap_or(0),
            latest.map_or(0, |s| s.requested_frames),
            latest.map_or(0, |s| match s.outcome {
                VirtualCallbackOutcome::Unfinished => 0,
                VirtualCallbackOutcome::Success => 1,
                VirtualCallbackOutcome::Error => 2,
            }),
            latest.and_then(|s| s.qpc_before_100ns).unwrap_or(0),
            latest.and_then(|s| s.qpc_after_100ns).unwrap_or(0),
            latest.and_then(|s| s.duration_100ns).unwrap_or(0),
        ];
        self.sequence.store(version + 1, SeqCst);
        for (slot, value) in self.fields.iter().zip(fields) {
            slot.store(value, SeqCst);
        }
        self.sequence.store(next, SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn publish_for_test(&self, value: VirtualCallbackSnapshot) {
        self.publish(value);
    }

    pub(crate) fn snapshot(&self) -> Option<VirtualCallbackSnapshot> {
        let before = self.sequence.load(SeqCst);
        if before == 0 || !before.is_multiple_of(2) {
            return None;
        }
        let f: [u64; FIELDS] = std::array::from_fn(|i| self.fields[i].load(SeqCst));
        if before != self.sequence.load(SeqCst) {
            return None;
        }
        let present = |bit: u32| f[7] & (1_u64 << bit) != 0_u64;
        Some(VirtualCallbackSnapshot {
            attempts: f[0],
            completed: f[1],
            errors: f[2],
            valid_timings: f[3],
            missing_clocks: f[4],
            regressed_clocks: f[5],
            max_duration_100ns: present(1).then_some(f[6]),
            saturated: present(0),
            latest: present(2).then(|| VirtualCallbackSample {
                attempt_sequence: f[8],
                epoch_before: f[9],
                frame_before: f[10],
                epoch_after: present(3).then_some(f[11]),
                frame_after: present(4).then_some(f[12]),
                requested_frames: f[13],
                outcome: match f[14] {
                    1 => VirtualCallbackOutcome::Success,
                    2 => VirtualCallbackOutcome::Error,
                    _ => VirtualCallbackOutcome::Unfinished,
                },
                qpc_before_100ns: present(5).then_some(f[15]),
                qpc_after_100ns: present(6).then_some(f[16]),
                duration_100ns: present(7).then_some(f[17]),
            }),
        })
    }
}

#[cfg(windows)]
#[derive(Default)]
pub(crate) struct CallbackTiming {
    value: VirtualCallbackSnapshot,
    previous_after: Option<u64>,
}

#[cfg(any(windows, test))]
fn add_counter(value: &mut u64, saturated: &mut bool) {
    *value = value.checked_add(1).unwrap_or_else(|| {
        *saturated = true;
        u64::MAX
    });
}

#[cfg(windows)]
impl CallbackTiming {
    pub(crate) fn render<S: ProcessingSample>(
        &mut self,
        engine: &mut Engine<S>,
        frames: usize,
        probe: Option<&VirtualCallbackProbe>,
        now: &mut impl FnMut() -> Option<u64>,
    ) -> Result<RenderReport, RuntimeError> {
        let Some(probe) = probe else {
            return engine.render(frames);
        };
        add_counter(&mut self.value.attempts, &mut self.value.saturated);
        let mut latest = VirtualCallbackSample {
            attempt_sequence: self.value.attempts,
            epoch_before: engine.epoch(),
            frame_before: engine.timeline(),
            epoch_after: None,
            frame_after: None,
            requested_frames: frames as u64,
            outcome: VirtualCallbackOutcome::Unfinished,
            qpc_before_100ns: None,
            qpc_after_100ns: None,
            duration_100ns: None,
        };
        // Publish before timing. A panic leaves an explicit unfinished attempt;
        // no invented completion clock or post-render Engine coordinates.
        self.value.latest = Some(latest);
        probe.publish(self.value);
        let before = now().filter(|&value| value != 0);
        let result = engine.render(frames);
        let after = now().filter(|&value| value != 0);
        latest.epoch_after = Some(engine.epoch());
        latest.frame_after = Some(engine.timeline());
        latest.qpc_before_100ns = before;
        latest.qpc_after_100ns = after;
        latest.outcome = if result.is_ok() {
            VirtualCallbackOutcome::Success
        } else {
            VirtualCallbackOutcome::Error
        };
        add_counter(&mut self.value.completed, &mut self.value.saturated);
        if result.is_err() {
            add_counter(&mut self.value.errors, &mut self.value.saturated);
        }
        let regressed = before.zip(after).is_some_and(|(a, b)| b < a)
            || self.previous_after.zip(before).is_some_and(|(a, b)| b < a);
        if before.is_none() || after.is_none() {
            add_counter(&mut self.value.missing_clocks, &mut self.value.saturated);
        }
        if regressed {
            add_counter(&mut self.value.regressed_clocks, &mut self.value.saturated);
        }
        if !regressed && let Some(duration) = before.zip(after).and_then(|(a, b)| b.checked_sub(a))
        {
            latest.duration_100ns = Some(duration);
            add_counter(&mut self.value.valid_timings, &mut self.value.saturated);
            self.value.max_duration_100ns =
                Some(self.value.max_duration_100ns.unwrap_or(0).max(duration));
        }
        self.previous_after = after;
        self.value.latest = Some(latest);
        probe.publish(self.value);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unfinished(attempt_sequence: u64) -> VirtualCallbackSample {
        VirtualCallbackSample {
            attempt_sequence,
            epoch_before: 3,
            frame_before: 128,
            epoch_after: None,
            frame_after: None,
            requested_frames: 64,
            outcome: VirtualCallbackOutcome::Unfinished,
            qpc_before_100ns: None,
            qpc_after_100ns: None,
            duration_100ns: None,
        }
    }

    #[test]
    fn callback_publication_roundtrips_options_and_replaces_old_completion_without_heap_work() {
        let probe = VirtualCallbackProbe::new();
        assert!(probe.snapshot().is_none());
        let complete = VirtualCallbackSnapshot {
            attempts: 1,
            completed: 1,
            valid_timings: 1,
            max_duration_100ns: Some(0),
            latest: Some(VirtualCallbackSample {
                epoch_after: Some(3),
                frame_after: Some(192),
                qpc_before_100ns: Some(100),
                qpc_after_100ns: Some(100),
                duration_100ns: Some(0),
                outcome: VirtualCallbackOutcome::Success,
                ..unfinished(1)
            }),
            ..Default::default()
        };
        crate::test_allocation::assert_allocates_nothing(|| {
            probe.publish(complete);
            assert_eq!(probe.snapshot(), Some(complete));
            let pending = VirtualCallbackSnapshot {
                attempts: 2,
                latest: Some(unfinished(2)),
                ..complete
            };
            probe.publish(pending);
            assert_eq!(probe.snapshot(), Some(pending));
            let mut failed = pending;
            failed.completed = 2;
            failed.errors = 1;
            failed.latest.as_mut().unwrap().outcome = VirtualCallbackOutcome::Error;
            probe.publish(failed);
            assert_eq!(probe.snapshot(), Some(failed));
        });
    }

    #[test]
    fn callback_contention_and_version_exhaustion_remain_unknown_without_retry() {
        let probe = VirtualCallbackProbe::new();
        probe.sequence.store(1, SeqCst);
        assert!(probe.snapshot().is_none());
        probe.sequence.store(u64::MAX - 1, SeqCst);
        probe.publish(VirtualCallbackSnapshot::default());
        assert!(probe.snapshot().is_none());
        probe.publish(VirtualCallbackSnapshot::default());
        assert!(probe.snapshot().is_none());
    }

    #[test]
    fn callback_counter_saturation_is_sticky_and_never_wraps_to_a_clean_zero() {
        let mut counter = u64::MAX - 1;
        let mut saturated = false;
        add_counter(&mut counter, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(!saturated);
        add_counter(&mut counter, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(saturated);
        add_counter(&mut counter, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(saturated);
        let probe = VirtualCallbackProbe::new();
        let sample = VirtualCallbackSnapshot {
            attempts: counter,
            saturated,
            ..Default::default()
        };
        probe.publish(sample);
        assert_eq!(probe.snapshot(), Some(sample));
    }
}
