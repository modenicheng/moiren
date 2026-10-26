//! Bounded VAD PCM worker telemetry.
//!
//! The worker updates atomics at native packet boundaries.  The control side
//! reads a copy after the worker has drained, so no audio callback, file IO,
//! JSON serialization, or growing collection is involved in the hot path.

use serde::Serialize;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct VadPcmTelemetrySnapshot {
    pub schema_version: u32,
    pub packet_frames: u32,
    pub max_packet_frames: u32,
    pub submission_attempts: u64,
    pub submitted_packets: u64,
    pub completed_packets: u64,
    pub completion_gaps: u64,
    pub max_completion_gap_100ns: u64,
    pub completion_clock_regressions: u64,
    pub dropped_packets: u64,
    pub driver_dropped_packets: u64,
    pub quota_dropped_packets: u64,
    pub carrier_dropped_packets: u64,
    pub stale_packets: u64,
    pub silence_packets: u64,
    pub silence_frames: u64,
    pub epoch_resets: u64,
    pub recovery_events: u64,
}

struct Counters {
    packet_frames: AtomicU32,
    max_packet_frames: AtomicU32,
    submission_attempts: AtomicU64,
    submitted_packets: AtomicU64,
    completed_packets: AtomicU64,
    completion_gaps: AtomicU64,
    max_completion_gap_100ns: AtomicU64,
    completion_clock_regressions: AtomicU64,
    dropped_packets: AtomicU64,
    driver_dropped_packets: AtomicU64,
    quota_dropped_packets: AtomicU64,
    carrier_dropped_packets: AtomicU64,
    stale_packets: AtomicU64,
    silence_packets: AtomicU64,
    silence_frames: AtomicU64,
    epoch_resets: AtomicU64,
    recovery_events: AtomicU64,
    last_completion_100ns: AtomicU64,
    last_epoch: AtomicU64,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            packet_frames: AtomicU32::new(0),
            max_packet_frames: AtomicU32::new(0),
            submission_attempts: AtomicU64::new(0),
            submitted_packets: AtomicU64::new(0),
            completed_packets: AtomicU64::new(0),
            completion_gaps: AtomicU64::new(0),
            max_completion_gap_100ns: AtomicU64::new(0),
            completion_clock_regressions: AtomicU64::new(0),
            dropped_packets: AtomicU64::new(0),
            driver_dropped_packets: AtomicU64::new(0),
            quota_dropped_packets: AtomicU64::new(0),
            carrier_dropped_packets: AtomicU64::new(0),
            stale_packets: AtomicU64::new(0),
            silence_packets: AtomicU64::new(0),
            silence_frames: AtomicU64::new(0),
            epoch_resets: AtomicU64::new(0),
            recovery_events: AtomicU64::new(0),
            last_completion_100ns: AtomicU64::new(0),
            last_epoch: AtomicU64::new(0),
        }
    }
}

/// A fixed-shape worker/control handoff.  All updates are saturating so a
/// long-running session cannot turn a diagnostic overflow into false evidence.
pub(super) struct PcmTelemetry {
    counters: Counters,
    clock_origin: Instant,
}

impl Default for PcmTelemetry {
    fn default() -> Self {
        Self {
            counters: Counters::default(),
            clock_origin: Instant::now(),
        }
    }
}

impl PcmTelemetry {
    pub(super) fn packet(&self, frames: u32) {
        if frames == 0 {
            return;
        }
        self.counters.packet_frames.store(frames, Ordering::Relaxed);
        max_u32(&self.counters.max_packet_frames, frames);
    }

    pub(super) fn submission_attempt(&self, frames: u32, silence_frames: usize) {
        self.packet(frames);
        saturating_add(&self.counters.submission_attempts, 1);
        if silence_frames != 0 {
            saturating_add(&self.counters.silence_packets, 1);
            saturating_add(
                &self.counters.silence_frames,
                u64::try_from(silence_frames).unwrap_or(u64::MAX),
            );
        }
    }

    pub(super) fn submission_accepted(&self) {
        saturating_add(&self.counters.submitted_packets, 1);
    }

    pub(super) fn completion(&self, frames: u32) {
        self.record_completion(frames, self.elapsed_100ns());
    }

    #[cfg(test)]
    pub(super) fn completion_at(&self, frames: u32, now_100ns: u64) {
        self.record_completion(frames, now_100ns);
    }

    fn record_completion(&self, frames: u32, now_100ns: u64) {
        self.packet(frames);
        saturating_add(&self.counters.completed_packets, 1);
        let previous = self
            .counters
            .last_completion_100ns
            .swap(now_100ns, Ordering::Relaxed);
        if previous == 0 {
            return;
        }
        if let Some(gap) = now_100ns.checked_sub(previous) {
            saturating_add(&self.counters.completion_gaps, 1);
            max_u64(&self.counters.max_completion_gap_100ns, gap);
        } else {
            saturating_add(&self.counters.completion_clock_regressions, 1);
        }
    }

    pub(super) fn driver_drop(&self) {
        saturating_add(&self.counters.dropped_packets, 1);
        saturating_add(&self.counters.driver_dropped_packets, 1);
    }

    pub(super) fn quota_drop(&self) {
        saturating_add(&self.counters.dropped_packets, 1);
        saturating_add(&self.counters.quota_dropped_packets, 1);
    }

    pub(super) fn carrier_drop(&self) {
        saturating_add(&self.counters.dropped_packets, 1);
        saturating_add(&self.counters.carrier_dropped_packets, 1);
    }

    pub(super) fn stale_packet(&self) {
        saturating_add(&self.counters.stale_packets, 1);
    }

    pub(super) fn epoch_reset(&self, epoch: u64) {
        if epoch == 0 {
            return;
        }
        let mut previous = self.counters.last_epoch.load(Ordering::Relaxed);
        loop {
            if epoch <= previous {
                return;
            }
            match self.counters.last_epoch.compare_exchange_weak(
                previous,
                epoch,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    saturating_add(&self.counters.epoch_resets, 1);
                    return;
                }
                Err(observed) => previous = observed,
            }
        }
    }

    pub(super) fn recovery(&self) {
        saturating_add(&self.counters.recovery_events, 1);
    }

    pub(super) fn snapshot(&self) -> VadPcmTelemetrySnapshot {
        VadPcmTelemetrySnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            packet_frames: self.counters.packet_frames.load(Ordering::Relaxed),
            max_packet_frames: self.counters.max_packet_frames.load(Ordering::Relaxed),
            submission_attempts: self.counters.submission_attempts.load(Ordering::Relaxed),
            submitted_packets: self.counters.submitted_packets.load(Ordering::Relaxed),
            completed_packets: self.counters.completed_packets.load(Ordering::Relaxed),
            completion_gaps: self.counters.completion_gaps.load(Ordering::Relaxed),
            max_completion_gap_100ns: self
                .counters
                .max_completion_gap_100ns
                .load(Ordering::Relaxed),
            completion_clock_regressions: self
                .counters
                .completion_clock_regressions
                .load(Ordering::Relaxed),
            dropped_packets: self.counters.dropped_packets.load(Ordering::Relaxed),
            driver_dropped_packets: self.counters.driver_dropped_packets.load(Ordering::Relaxed),
            quota_dropped_packets: self.counters.quota_dropped_packets.load(Ordering::Relaxed),
            carrier_dropped_packets: self
                .counters
                .carrier_dropped_packets
                .load(Ordering::Relaxed),
            stale_packets: self.counters.stale_packets.load(Ordering::Relaxed),
            silence_packets: self.counters.silence_packets.load(Ordering::Relaxed),
            silence_frames: self.counters.silence_frames.load(Ordering::Relaxed),
            epoch_resets: self.counters.epoch_resets.load(Ordering::Relaxed),
            recovery_events: self.counters.recovery_events.load(Ordering::Relaxed),
        }
    }

    fn elapsed_100ns(&self) -> u64 {
        let nanos = Instant::now()
            .saturating_duration_since(self.clock_origin)
            .as_nanos()
            .div_ceil(100);
        u64::try_from(nanos).unwrap_or(u64::MAX).max(1)
    }
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    let mut previous = counter.load(Ordering::Relaxed);
    loop {
        let next = previous.saturating_add(value);
        match counter.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => previous = observed,
        }
    }
}

fn max_u32(counter: &AtomicU32, value: u32) {
    let mut previous = counter.load(Ordering::Relaxed);
    while previous < value {
        match counter.compare_exchange_weak(previous, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => previous = observed,
        }
    }
}

fn max_u64(counter: &AtomicU64, value: u64) {
    let mut previous = counter.load(Ordering::Relaxed);
    while previous < value {
        match counter.compare_exchange_weak(previous, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => previous = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_records_packet_gap_drop_silence_epoch_and_recovery_without_storage_growth() {
        let telemetry = PcmTelemetry::default();
        telemetry.submission_attempt(480, 32);
        telemetry.submission_accepted();
        telemetry.completion_at(480, 1_000);
        telemetry.completion_at(240, 2_250);
        telemetry.driver_drop();
        telemetry.quota_drop();
        telemetry.carrier_drop();
        telemetry.stale_packet();
        telemetry.epoch_reset(1);
        telemetry.epoch_reset(1);
        telemetry.epoch_reset(2);
        telemetry.recovery();

        assert_eq!(
            telemetry.snapshot(),
            VadPcmTelemetrySnapshot {
                schema_version: SNAPSHOT_SCHEMA_VERSION,
                packet_frames: 240,
                max_packet_frames: 480,
                submission_attempts: 1,
                submitted_packets: 1,
                completed_packets: 2,
                completion_gaps: 1,
                max_completion_gap_100ns: 1_250,
                completion_clock_regressions: 0,
                dropped_packets: 3,
                driver_dropped_packets: 1,
                quota_dropped_packets: 1,
                carrier_dropped_packets: 1,
                stale_packets: 1,
                silence_packets: 1,
                silence_frames: 32,
                epoch_resets: 2,
                recovery_events: 1,
            }
        );
    }

    #[test]
    fn regressed_completion_clock_is_counted_and_does_not_fabricate_a_gap() {
        let telemetry = PcmTelemetry::default();
        telemetry.completion_at(64, 2_000);
        telemetry.completion_at(64, 1_999);
        let snapshot = telemetry.snapshot();

        assert_eq!(snapshot.completion_gaps, 0);
        assert_eq!(snapshot.max_completion_gap_100ns, 0);
        assert_eq!(snapshot.completion_clock_regressions, 1);
    }
}
