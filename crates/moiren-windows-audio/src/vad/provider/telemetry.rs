//! Bounded VAD PCM worker telemetry.
//!
//! The worker updates atomics at native packet boundaries.  The control side
//! reads a copy after the worker has drained, so no audio callback, file IO,
//! JSON serialization, or growing collection is involved in the hot path.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

const SNAPSHOT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct VadPcmTelemetrySnapshot {
    #[serde(
        default = "current_schema_version",
        deserialize_with = "deserialize_current_schema_version"
    )]
    pub schema_version: u32,
    pub packet_frames: u32,
    pub max_packet_frames: u32,
    pub submission_attempts: u64,
    pub submitted_packets: u64,
    pub completed_packets: u64,
    #[serde(alias = "completion_gaps")]
    pub completion_interval_observations: u64,
    #[serde(alias = "max_completion_gap_100ns")]
    pub max_completion_interval_100ns: u64,
    pub completion_clock_regressions: u64,
    /// Playback READ native forward sequence gaps observed after a known
    /// predecessor. The unknown first prefix is not inferred or counted.
    pub sequence_gap_observations: u64,
    /// Missing playback native sequence slots, not proven non-silent PCM drops.
    /// No frame conversion or native sample age follows from this count.
    pub missing_sequence_slots: u64,
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

fn current_schema_version() -> u32 {
    SNAPSHOT_SCHEMA_VERSION
}

fn deserialize_current_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(u32::deserialize(deserializer)?.max(SNAPSHOT_SCHEMA_VERSION))
}

struct Counters {
    packet_frames: AtomicU32,
    max_packet_frames: AtomicU32,
    submission_attempts: AtomicU64,
    submitted_packets: AtomicU64,
    completed_packets: AtomicU64,
    completion_intervals: AtomicU64,
    max_completion_interval_100ns: AtomicU64,
    completion_clock_regressions: AtomicU64,
    sequence_gap_observations: AtomicU64,
    missing_sequence_slots: AtomicU64,
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
            completion_intervals: AtomicU64::new(0),
            max_completion_interval_100ns: AtomicU64::new(0),
            completion_clock_regressions: AtomicU64::new(0),
            sequence_gap_observations: AtomicU64::new(0),
            missing_sequence_slots: AtomicU64::new(0),
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
            saturating_add(&self.counters.completion_intervals, 1);
            max_u64(&self.counters.max_completion_interval_100ns, gap);
        } else {
            saturating_add(&self.counters.completion_clock_regressions, 1);
        }
    }

    pub(super) fn driver_drop(&self) {
        saturating_add(&self.counters.dropped_packets, 1);
        saturating_add(&self.counters.driver_dropped_packets, 1);
    }

    /// Caller supplies only accepted playback READ native forward gaps; a
    /// microphone completion echoes host submission order, not native supply.
    /// The caller ignores an unknown first prefix; zero adds no observation.
    pub(super) fn sequence_gap(&self, missing_slots: u64) {
        if missing_slots == 0 {
            return;
        }
        saturating_add(&self.counters.sequence_gap_observations, 1);
        saturating_add(&self.counters.missing_sequence_slots, missing_slots);
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
            completion_interval_observations: self
                .counters
                .completion_intervals
                .load(Ordering::Relaxed),
            max_completion_interval_100ns: self
                .counters
                .max_completion_interval_100ns
                .load(Ordering::Relaxed),
            completion_clock_regressions: self
                .counters
                .completion_clock_regressions
                .load(Ordering::Relaxed),
            sequence_gap_observations: self
                .counters
                .sequence_gap_observations
                .load(Ordering::Relaxed),
            missing_sequence_slots: self.counters.missing_sequence_slots.load(Ordering::Relaxed),
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
    fn snapshot_records_packet_intervals_drop_silence_epoch_and_recovery_without_storage_growth() {
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
                completion_interval_observations: 1,
                max_completion_interval_100ns: 1_250,
                completion_clock_regressions: 0,
                sequence_gap_observations: 0,
                missing_sequence_slots: 0,
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
    fn regressed_completion_clock_is_counted_without_an_interval_observation() {
        let telemetry = PcmTelemetry::default();
        telemetry.completion_at(64, 2_000);
        telemetry.completion_at(64, 1_999);
        let snapshot = telemetry.snapshot();

        assert_eq!(snapshot.completion_interval_observations, 0);
        assert_eq!(snapshot.max_completion_interval_100ns, 0);
        assert_eq!(snapshot.completion_clock_regressions, 1);
    }

    #[test]
    fn legacy_completion_gaps_json_deserializes_with_the_corrected_field() {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "completion_gaps": 40,
            "max_completion_gap_100ns": 110_118
        });
        let snapshot: VadPcmTelemetrySnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(snapshot.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.completion_interval_observations, 40);
        assert_eq!(snapshot.max_completion_interval_100ns, 110_118);

        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized["schema_version"], SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(serialized["completion_interval_observations"], 40);
        assert_eq!(serialized["max_completion_interval_100ns"], 110_118);
        assert!(serialized.get("completion_gaps").is_none());
        assert!(serialized.get("max_completion_gap_100ns").is_none());
    }

    #[test]
    fn sequence_gap_schema_three_defaults_new_fields_for_legacy_json() {
        for legacy in [
            serde_json::json!({}),
            serde_json::json!({ "schema_version": 0 }),
            serde_json::json!({ "schema_version": 1, "completion_gaps": 40 }),
            serde_json::json!({ "schema_version": 2, "dropped_packets": 7 }),
        ] {
            let snapshot: VadPcmTelemetrySnapshot = serde_json::from_value(legacy).unwrap();
            let serialized = serde_json::to_value(snapshot).unwrap();
            assert_eq!(serialized["schema_version"], 3);
            assert_eq!(serialized["sequence_gap_observations"], 0);
            assert_eq!(serialized["missing_sequence_slots"], 0);
            assert!(serialized.get("completion_gaps").is_none());
        }
        let serialized = serde_json::to_value(PcmTelemetry::default().snapshot()).unwrap();
        assert_eq!(serialized["schema_version"], 3);
        assert_eq!(serialized["sequence_gap_observations"], 0);
        assert_eq!(serialized["missing_sequence_slots"], 0);
    }

    #[test]
    fn sequence_gap_json_keeps_u64_precision_and_future_schema_versions() {
        let large = (1_u64 << 53) + 1;
        let json = format!(
            r#"{{"schema_version":7,"sequence_gap_observations":{large},"missing_sequence_slots":{}}}"#,
            u64::MAX,
        );
        let snapshot: VadPcmTelemetrySnapshot = serde_json::from_str(&json).unwrap();
        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized["schema_version"], 7);
        assert_eq!(
            serialized["sequence_gap_observations"].as_u64(),
            Some(large)
        );
        assert_eq!(
            serialized["missing_sequence_slots"].as_u64(),
            Some(u64::MAX)
        );
        assert_eq!(serialized["dropped_packets"], 0);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(encoded.contains(&format!(r#""sequence_gap_observations":{large}"#)));
        assert!(encoded.contains(&format!(r#""missing_sequence_slots":{}"#, u64::MAX)));
    }

    #[test]
    fn sequence_gap_counts_playback_forward_observations_without_claiming_pcm_drops() {
        let telemetry = PcmTelemetry::default();
        let before = telemetry.snapshot();
        telemetry.sequence_gap(0);
        assert_eq!(telemetry.snapshot(), before);

        telemetry.sequence_gap(2);
        telemetry.sequence_gap(5);
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.sequence_gap_observations, 2);
        assert_eq!(snapshot.missing_sequence_slots, 7);
        assert_eq!(snapshot.dropped_packets, 0);
        assert_eq!(snapshot.driver_dropped_packets, 0);
        assert_eq!(snapshot.quota_dropped_packets, 0);
        assert_eq!(snapshot.carrier_dropped_packets, 0);
        assert_eq!(snapshot.stale_packets, 0);
        assert_eq!(snapshot.packet_frames, 0);
        assert_eq!(snapshot.completed_packets, 0);
        assert_eq!(snapshot.silence_frames, 0);
        assert_eq!(snapshot.epoch_resets, 0);
        assert_eq!(snapshot.recovery_events, 0);
    }

    #[test]
    fn sequence_gap_update_serializes_large_integer_slot_counts_exactly() {
        let telemetry = PcmTelemetry::default();
        let large = (1_u64 << 53) + 1;
        telemetry.sequence_gap(large);
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.sequence_gap_observations, 1);
        assert_eq!(snapshot.missing_sequence_slots, large);
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(&format!(r#""missing_sequence_slots":{large}"#)));
        let decoded: VadPcmTelemetrySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn sequence_gap_counters_saturate_independently_without_changing_drop_counters() {
        let telemetry = PcmTelemetry::default();
        telemetry
            .counters
            .sequence_gap_observations
            .store(u64::MAX - 1, Ordering::Relaxed);
        telemetry
            .counters
            .missing_sequence_slots
            .store(u64::MAX - 2, Ordering::Relaxed);
        telemetry.driver_drop();
        telemetry.sequence_gap(2);
        let at_limit = telemetry.snapshot();
        assert_eq!(at_limit.sequence_gap_observations, u64::MAX);
        assert_eq!(at_limit.missing_sequence_slots, u64::MAX);
        telemetry.sequence_gap(u64::MAX);
        telemetry.sequence_gap(0);
        assert_eq!(telemetry.snapshot(), at_limit);
        assert_eq!(at_limit.dropped_packets, 1);
        assert_eq!(at_limit.driver_dropped_packets, 1);

        let telemetry = PcmTelemetry::default();
        telemetry
            .counters
            .sequence_gap_observations
            .store(u64::MAX, Ordering::Relaxed);
        telemetry.sequence_gap(3);
        assert_eq!(telemetry.snapshot().sequence_gap_observations, u64::MAX);
        assert_eq!(telemetry.snapshot().missing_sequence_slots, 3);

        let telemetry = PcmTelemetry::default();
        telemetry
            .counters
            .missing_sequence_slots
            .store(u64::MAX, Ordering::Relaxed);
        telemetry.sequence_gap(3);
        assert_eq!(telemetry.snapshot().sequence_gap_observations, 1);
        assert_eq!(telemetry.snapshot().missing_sequence_slots, u64::MAX);
    }
}
