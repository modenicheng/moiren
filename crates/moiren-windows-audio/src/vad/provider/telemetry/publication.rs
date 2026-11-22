//! Decode-call envelopes, not atomic publication instants or source age.
//! One PCM worker writes; a live reader makes one coherent attempt, never spins.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PublicationIdentity {
    /// Original PcmOwner identity, never a later QUERY or revision.
    pub native_stamp: [u64; 5],
    pub reset_epoch: u64,
    pub admission_ticket: u64,
    pub active_revision: u64,
    pub format: u32,
    pub channels: u32,
    pub packet_frames: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PublicationSnapshot {
    pub identity: Option<PublicationIdentity>,
    pub qpc_frequency_hz: Option<u64>,
    pub successful_decode_calls: u64,
    /// Frames represented by Ok(true), NOT confirmed carrier publication.
    pub decode_ok_frames: u64,
    /// The existing carrier API cannot witness an abandonment race's effect.
    pub carrier_publication_effect_known: Option<bool>,
    pub valid_call_intervals: u64,
    pub max_call_interval_100ns: Option<u64>,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub first_before_qpc: Option<u64>,
    pub last_after_qpc: Option<u64>,
    pub carrier_full_packets: u64,
    pub decode_error_packets: u64,
    pub stale_packets: u64,
    pub missing_sequence_slots: u64,
    pub ambiguous_packets: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
    pub conversion_overflows: u64,
    pub segment_breaks: u64,
    pub owner_reopens: u64,
    pub invalidated: bool,
    pub saturated: bool,
    /// Always unknown: no prefix/suffix fence, consumer trace or join witness.
    pub complete_delivery_coverage: Option<bool>,
}

pub(in crate::vad::provider) struct PublicationRecorder {
    version: AtomicU64,
    bound: AtomicBool,
    identity: [AtomicU64; 11],
    frequency: Option<u64>,
    successful_decode_calls: AtomicU64,
    decode_ok_frames: AtomicU64,
    intervals: AtomicU64,
    maximum: AtomicU64,
    first_sequence: AtomicU64,
    last_sequence: AtomicU64,
    first_before: AtomicU64,
    last_after: AtomicU64,
    previous_before: AtomicU64,
    previous_after: AtomicU64,
    carrier_full: AtomicU64,
    decode_errors: AtomicU64,
    stale: AtomicU64,
    missing: AtomicU64,
    ambiguous: AtomicU64,
    unavailable: AtomicU64,
    regressed: AtomicU64,
    overflows: AtomicU64,
    breaks: AtomicU64,
    reopens: AtomicU64,
    invalidated: AtomicBool,
    saturated: AtomicBool,
}

impl PublicationRecorder {
    pub(in crate::vad::provider) fn new(frequency: Option<u64>) -> Self {
        Self {
            version: AtomicU64::new(0),
            bound: AtomicBool::new(false),
            identity: std::array::from_fn(|_| AtomicU64::new(0)),
            frequency: frequency.filter(|&n| n != 0),
            successful_decode_calls: AtomicU64::new(0),
            decode_ok_frames: AtomicU64::new(0),
            intervals: AtomicU64::new(0),
            maximum: AtomicU64::new(0),
            first_sequence: AtomicU64::new(0),
            last_sequence: AtomicU64::new(0),
            first_before: AtomicU64::new(0),
            last_after: AtomicU64::new(0),
            previous_before: AtomicU64::new(0),
            previous_after: AtomicU64::new(0),
            carrier_full: AtomicU64::new(0),
            decode_errors: AtomicU64::new(0),
            stale: AtomicU64::new(0),
            missing: AtomicU64::new(0),
            ambiguous: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            regressed: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            breaks: AtomicU64::new(0),
            reopens: AtomicU64::new(0),
            invalidated: AtomicBool::new(false),
            saturated: AtomicBool::new(false),
        }
    }

    // All mutations run on the sole PCM worker. SeqCst covers every scalar so
    // a reader cannot mistake a mixed live summary for a coherent one.
    fn begin(&self) -> bool {
        let version = self.version.load(SeqCst);
        if version > u64::MAX - 2 {
            return false;
        }
        self.version.store(version + 1, SeqCst);
        true
    }
    fn end(&self) {
        self.version.store(self.version.load(SeqCst) + 1, SeqCst);
    }
    fn add(&self, counter: &AtomicU64, n: u64) {
        let previous = counter.load(SeqCst);
        match previous.checked_add(n) {
            Some(next) => counter.store(next, SeqCst),
            None => {
                counter.store(u64::MAX, SeqCst);
                self.saturated.store(true, SeqCst);
            }
        }
    }
    fn break_segment(&self) {
        if self.previous_before.swap(0, SeqCst) != 0 {
            self.add(&self.breaks, 1);
        }
        self.previous_after.store(0, SeqCst);
    }
    pub(in crate::vad::provider) fn bind(&self, identity: PublicationIdentity) {
        if !self.begin() {
            return;
        }
        if self.bound.load(SeqCst) {
            self.add(&self.reopens, 1);
            self.invalidated.store(true, SeqCst);
            self.break_segment();
        } else {
            let values = [
                identity.native_stamp[0],
                identity.native_stamp[1],
                identity.native_stamp[2],
                identity.native_stamp[3],
                identity.native_stamp[4],
                identity.reset_epoch,
                identity.admission_ticket,
                identity.active_revision,
                u64::from(identity.format),
                u64::from(identity.channels),
                u64::from(identity.packet_frames),
            ];
            for (slot, value) in self.identity.iter().zip(values) {
                slot.store(value, SeqCst);
            }
            self.bound.store(true, SeqCst);
        }
        self.end();
    }
    pub(in crate::vad::provider) fn matches(&self, epoch: u64, ticket: u64, revision: u64) -> bool {
        self.bound.load(SeqCst)
            && !self.invalidated.load(SeqCst)
            && self.identity[5].load(SeqCst) == epoch
            && self.identity[6].load(SeqCst) == ticket
            && self.identity[7].load(SeqCst) == revision
    }
    pub(in crate::vad::provider) fn invalidate(&self) {
        if !self.bound.load(SeqCst) || !self.begin() {
            return;
        }
        self.invalidated.store(true, SeqCst);
        self.break_segment();
        self.end();
    }
    pub(in crate::vad::provider) fn stale(&self) {
        if self.begin() {
            self.add(&self.stale, 1);
            self.end();
        }
    }
    pub(in crate::vad::provider) fn missing(&self, slots: u64) {
        if self.begin() {
            self.add(&self.missing, slots);
            self.end();
        }
    }
    pub(in crate::vad::provider) fn record(
        &self,
        before: Option<u64>,
        after: Option<u64>,
        result: Result<bool, ()>,
        frames: u32,
        sequence: u64,
        valid: bool,
    ) {
        if !self.begin() {
            return;
        }
        if !valid {
            self.invalidated.store(true, SeqCst);
            self.break_segment();
        }
        match result {
            Ok(false) => self.add(&self.carrier_full, 1),
            Err(()) => self.add(&self.decode_errors, 1),
            Ok(true) => {
                self.add(&self.successful_decode_calls, 1);
                self.add(&self.decode_ok_frames, u64::from(frames));
                if !valid || !self.bound.load(SeqCst) || self.invalidated.load(SeqCst) {
                    self.add(&self.ambiguous, 1);
                    self.invalidated.store(true, SeqCst);
                    self.break_segment();
                } else {
                    self.record_envelope(before, after, sequence);
                }
            }
        }
        // Ordinary unsuccessful calls do not erase the last success; the next
        // interval must still expose the full starvation gap across them.
        self.end();
    }
    fn record_envelope(&self, before: Option<u64>, after: Option<u64>, sequence: u64) {
        let Some((before, after, frequency)) = before
            .filter(|&n| n != 0)
            .zip(after.filter(|&n| n != 0))
            .zip(self.frequency)
            .map(|((b, a), f)| (b, a, f))
        else {
            self.add(&self.unavailable, 1);
            self.break_segment();
            return;
        };
        if after < before
            || (self.previous_after.load(SeqCst) != 0 && before < self.previous_after.load(SeqCst))
        {
            self.add(&self.regressed, 1);
            self.break_segment();
            return;
        }
        let previous = self.previous_before.load(SeqCst);
        if previous != 0 {
            // Subtract raw SAME-domain ticks before conversion. These outer
            // call endpoints bound the inter-call interval, not publication.
            let converted =
                (u128::from(after - previous) * 10_000_000).div_ceil(u128::from(frequency));
            let Ok(gap) = u64::try_from(converted) else {
                self.add(&self.overflows, 1);
                self.break_segment();
                return;
            };
            self.add(&self.intervals, 1);
            self.maximum
                .store(self.maximum.load(SeqCst).max(gap), SeqCst);
        }
        if self.first_before.load(SeqCst) == 0 {
            self.first_before.store(before, SeqCst);
            self.first_sequence.store(sequence, SeqCst);
        }
        self.last_after.store(after, SeqCst);
        self.last_sequence.store(sequence, SeqCst);
        self.previous_before.store(before, SeqCst);
        self.previous_after.store(after, SeqCst);
    }
    pub(in crate::vad::provider) fn snapshot(&self) -> Option<PublicationSnapshot> {
        let version = self.version.load(SeqCst);
        if !version.is_multiple_of(2) || version > u64::MAX - 2 {
            return None;
        }
        let identity = self.bound.load(SeqCst).then(|| PublicationIdentity {
            native_stamp: std::array::from_fn(|i| self.identity[i].load(SeqCst)),
            reset_epoch: self.identity[5].load(SeqCst),
            admission_ticket: self.identity[6].load(SeqCst),
            active_revision: self.identity[7].load(SeqCst),
            format: self.identity[8].load(SeqCst) as u32,
            channels: self.identity[9].load(SeqCst) as u32,
            packet_frames: self.identity[10].load(SeqCst) as u32,
        });
        let intervals = self.intervals.load(SeqCst);
        let first = self.first_before.load(SeqCst);
        let snapshot = PublicationSnapshot {
            identity,
            qpc_frequency_hz: self.frequency,
            successful_decode_calls: self.successful_decode_calls.load(SeqCst),
            decode_ok_frames: self.decode_ok_frames.load(SeqCst),
            carrier_publication_effect_known: None,
            valid_call_intervals: intervals,
            max_call_interval_100ns: (intervals != 0).then(|| self.maximum.load(SeqCst)),
            first_sequence: (first != 0).then(|| self.first_sequence.load(SeqCst)),
            last_sequence: (first != 0).then(|| self.last_sequence.load(SeqCst)),
            first_before_qpc: (first != 0).then_some(first),
            last_after_qpc: (first != 0).then(|| self.last_after.load(SeqCst)),
            carrier_full_packets: self.carrier_full.load(SeqCst),
            decode_error_packets: self.decode_errors.load(SeqCst),
            stale_packets: self.stale.load(SeqCst),
            missing_sequence_slots: self.missing.load(SeqCst),
            ambiguous_packets: self.ambiguous.load(SeqCst),
            unavailable_clocks: self.unavailable.load(SeqCst),
            regressed_clocks: self.regressed.load(SeqCst),
            conversion_overflows: self.overflows.load(SeqCst),
            segment_breaks: self.breaks.load(SeqCst),
            owner_reopens: self.reopens.load(SeqCst),
            invalidated: self.invalidated.load(SeqCst),
            saturated: self.saturated.load(SeqCst),
            complete_delivery_coverage: None,
        };
        #[cfg(test)]
        BEFORE_SNAPSHOT_CHECK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        (version == self.version.load(SeqCst)).then_some(snapshot)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_TICKS: std::cell::RefCell<std::collections::VecDeque<Option<u64>>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static BEFORE_SNAPSHOT_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(in crate::vad::provider) fn test_tick() -> Option<Option<u64>> {
    TEST_TICKS.with(|ticks| ticks.borrow_mut().pop_front())
}

#[cfg(test)]
pub(in crate::vad::provider) fn set_test_ticks(ticks: &[Option<u64>]) {
    TEST_TICKS.with(|pending| *pending.borrow_mut() = ticks.iter().copied().collect());
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn identity() -> PublicationIdentity {
        PublicationIdentity {
            native_stamp: [1, 2, 3, 4, 5],
            reset_epoch: 0,
            admission_ticket: 2,
            active_revision: 1,
            format: 1,
            channels: 2,
            packet_frames: 144,
        }
    }

    #[test]
    fn publication_same_clock_outer_bound_ceil_and_unsuccessful_calls_retain_frontier() {
        let recorder = PublicationRecorder::new(Some(3_000_001));
        recorder.bind(identity());
        recorder.record(Some(100), Some(110), Ok(true), 144, 1, true);
        recorder.record(Some(120), Some(121), Ok(false), 144, 2, true);
        recorder.record(Some(130), Some(131), Err(()), 144, 3, true);
        recorder.record(Some(140), Some(150), Ok(true), 144, 4, true);
        let s = recorder.snapshot().unwrap();
        assert_eq!(s.successful_decode_calls, 2);
        assert_eq!(s.decode_ok_frames, 288);
        assert_eq!(s.valid_call_intervals, 1);
        assert_eq!(s.max_call_interval_100ns, Some(167));
        assert_eq!(s.carrier_full_packets, 1);
        assert_eq!(s.decode_error_packets, 1);
        assert_eq!(s.complete_delivery_coverage, None);
    }

    #[test]
    fn publication_clock_loss_regression_conversion_overflow_and_saturation_are_visible() {
        let recorder = PublicationRecorder::new(Some(1));
        recorder.bind(identity());
        recorder.record(Some(1), Some(2), Ok(true), 1, 1, true);
        recorder.record(None, Some(3), Ok(true), 1, 2, true);
        recorder.record(Some(4), Some(5), Ok(true), 1, 3, true);
        recorder.record(Some(3), Some(6), Ok(true), 1, 4, true);
        recorder.record(Some(7), Some(8), Ok(true), 1, 5, true);
        recorder.record(Some(9), Some(u64::MAX), Ok(true), 1, 6, true);
        let s = recorder.snapshot().unwrap();
        assert_eq!(s.unavailable_clocks, 1);
        assert_eq!(s.regressed_clocks, 1);
        assert_eq!(s.conversion_overflows, 1);
        assert_eq!(s.segment_breaks, 3);
        assert_eq!(s.valid_call_intervals, 0);
        assert_eq!(s.max_call_interval_100ns, None);
        recorder.decode_ok_frames.store(u64::MAX, SeqCst);
        recorder.record(Some(10), Some(11), Ok(true), 144, 7, true);
        assert!(recorder.snapshot().unwrap().saturated);
        recorder.version.store(u64::MAX - 1, SeqCst);
        recorder.stale();
        assert!(recorder.snapshot().is_none());
    }

    #[test]
    fn publication_reopen_keeps_original_identity_and_cannot_resume_old_epoch() {
        let recorder = PublicationRecorder::new(Some(10_000_000));
        recorder.bind(identity());
        recorder.record(Some(10), Some(11), Ok(true), 144, 1, true);
        recorder.bind(identity()); // Even a same-stamp new owner is a break.
        recorder.record(Some(20), Some(21), Ok(true), 144, 2, true);
        let s = recorder.snapshot().unwrap();
        assert_eq!(s.identity, Some(identity()));
        assert_eq!(s.owner_reopens, 1);
        assert_eq!(s.segment_breaks, 1);
        assert_eq!(s.ambiguous_packets, 1);
        assert_eq!(s.valid_call_intervals, 0);
        assert!(s.invalidated);
        assert!(!recorder.matches(0, 2, 1));
    }

    #[test]
    fn publication_unbound_contended_and_legacy_snapshots_never_establish_coverage() {
        let recorder = PublicationRecorder::new(None);
        recorder.invalidate(); // A startup reset cannot manufacture an owner.
        let s = recorder.snapshot().unwrap();
        assert_eq!(s.identity, None);
        assert_eq!(s.max_call_interval_100ns, None);
        assert!(!s.invalidated);
        recorder.version.store(1, SeqCst);
        assert!(recorder.snapshot().is_none());
        let legacy: crate::vad::provider::VadPcmTelemetrySnapshot =
            serde_json::from_str(r#"{"schema_version":4,"completed_packets":12}"#).unwrap();
        assert_eq!(legacy.playback_publication, None);
    }

    #[test]
    fn publication_snapshot_during_actual_writer_is_one_attempt_unknown() {
        use std::{
            sync::{Arc, mpsc},
            time::Duration,
        };
        let recorder = Arc::new(PublicationRecorder::new(Some(10_000_000)));
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let writer = recorder.clone();
        let worker = std::thread::spawn(move || {
            assert!(writer.begin());
            writer.add(&writer.successful_decode_calls, 1);
            entered_tx.send(()).unwrap();
            let released = release_rx.recv_timeout(Duration::from_secs(5));
            writer.add(&writer.decode_ok_frames, 144);
            writer.end();
            released.unwrap();
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(5));
        let observed = recorder.snapshot();
        let _ = release_tx.send(());
        worker.join().unwrap();
        entered.unwrap();
        assert!(observed.is_none());
        let final_snapshot = recorder.snapshot().unwrap();
        assert_eq!(final_snapshot.successful_decode_calls, 1);
        assert_eq!(final_snapshot.decode_ok_frames, 144);
        assert_eq!(final_snapshot.complete_delivery_coverage, None);
    }

    #[test]
    fn publication_snapshot_rejects_writer_advance_between_fields_and_version_check() {
        let recorder = std::sync::Arc::new(PublicationRecorder::new(Some(10_000_000)));
        recorder.bind(identity());
        recorder.record(Some(10), Some(11), Ok(true), 144, 1, true);
        let writer = recorder.clone();
        BEFORE_SNAPSHOT_CHECK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                writer.record(Some(20), Some(21), Ok(true), 144, 2, true);
            }));
        });
        assert!(recorder.snapshot().is_none());
        let snapshot = recorder.snapshot().unwrap();
        assert_eq!(snapshot.successful_decode_calls, 2);
        assert_eq!(snapshot.decode_ok_frames, 288);
        assert_eq!(snapshot.valid_call_intervals, 1);
        assert_eq!(snapshot.carrier_publication_effect_known, None);
    }
}
