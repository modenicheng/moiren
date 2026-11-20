//! Single-writer atomic DTO publication. Each endpoint owns its publisher;
//! there are no retries, locks, pointers into mutable storage, or RT drops.
use crate::{
    clock_bridge::{QueuedFrameAge, QueuedFrameStamp},
    latency::InputDeliverySnapshot,
};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

struct Sample<const N: usize> {
    sequence: AtomicU64,
    fields: [AtomicU64; N],
}

impl<const N: usize> Default for Sample<N> {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            fields: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl<const N: usize> Sample<N> {
    fn publish(&self, fields: [u64; N]) {
        // Exactly one writer per Sample: native ingress or ClockSource, never
        // both. SeqCst puts every scalar between the sequence markers in the
        // same total order as the reader. Atomic payloads avoid seqlock data
        // races. Exhaustion stays permanently unknown rather than wrapping ABA.
        let sequence = self.sequence.load(SeqCst);
        let Some(next) = sequence.checked_add(2) else {
            self.sequence.store(u64::MAX, SeqCst);
            return;
        };
        self.sequence.store(sequence + 1, SeqCst);
        for (slot, value) in self.fields.iter().zip(fields) {
            slot.store(value, SeqCst);
        }
        self.sequence.store(next, SeqCst);
    }

    fn snapshot(&self) -> Option<[u64; N]> {
        let before = self.sequence.load(SeqCst);
        if before == 0 || !before.is_multiple_of(2) {
            return None;
        }
        let fields = std::array::from_fn(|i| self.fields[i].load(SeqCst));
        (before == self.sequence.load(SeqCst)).then_some(fields)
    }
}

#[derive(Default)]
pub(in crate::clock_bridge) struct DeliverySample(Sample<9>);
impl DeliverySample {
    #[cfg(any(windows, test))]
    pub fn publish(&self, sample: InputDeliverySnapshot) {
        self.0.publish([
            sample.packets,
            u64::from(sample.max_packet_frames),
            sample.max_callback_burst_frames,
            sample.gap_samples,
            sample.max_gap_100ns,
            sample.packet_bound_violations,
            sample.gap_bound_violations,
            sample.unavailable_clocks,
            sample.regressed_clocks,
        ]);
    }
    pub fn snapshot(&self) -> Option<InputDeliverySnapshot> {
        let f = self.0.snapshot()?;
        Some(InputDeliverySnapshot {
            packets: f[0],
            max_packet_frames: f[1] as u32,
            max_callback_burst_frames: f[2],
            gap_samples: f[3],
            max_gap_100ns: f[4],
            packet_bound_violations: f[5],
            gap_bound_violations: f[6],
            unavailable_clocks: f[7],
            regressed_clocks: f[8],
        })
    }
}

#[derive(Default)]
pub(in crate::clock_bridge) struct AgeSample(Sample<12>);
impl AgeSample {
    pub fn publish(&self, sample: Option<QueuedFrameAge>) {
        let fields = sample.map_or([0; 12], |a| {
            [
                1,
                a.stamp.generation,
                a.stamp.packet_position_frames,
                a.stamp.packet_qpc_100ns,
                a.stamp.packet_accepted_frames,
                a.stamp.packet_offset_frames,
                u64::from(a.stamp.rate_correction_ppm.is_some()),
                a.stamp.rate_correction_ppm.unwrap_or(0) as u64,
                a.observed_qpc_100ns,
                u64::from(a.input_sample_rate),
                a.estimated_frame_qpc_100ns,
                a.estimated_age_100ns,
            ]
        });
        self.0.publish(fields);
    }
    pub fn snapshot(&self) -> Option<QueuedFrameAge> {
        let f = self.0.snapshot()?;
        (f[0] != 0).then_some(QueuedFrameAge {
            stamp: QueuedFrameStamp {
                generation: f[1],
                packet_position_frames: f[2],
                packet_qpc_100ns: f[3],
                packet_accepted_frames: f[4],
                packet_offset_frames: f[5],
                rate_correction_ppm: (f[6] != 0).then_some(f[7] as i64),
            },
            observed_qpc_100ns: f[8],
            input_sample_rate: f[9] as u32,
            estimated_frame_qpc_100ns: f[10],
            estimated_age_100ns: f[11],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn age(n: u64) -> QueuedFrameAge {
        QueuedFrameAge {
            stamp: QueuedFrameStamp {
                generation: n,
                packet_position_frames: n * 2,
                packet_qpc_100ns: n * 3,
                packet_accepted_frames: n * 4,
                packet_offset_frames: n * 5,
                rate_correction_ppm: Some(-(n as i64)),
            },
            observed_qpc_100ns: n * 6,
            input_sample_rate: (n * 7) as u32,
            estimated_frame_qpc_100ns: n * 8,
            estimated_age_100ns: n * 9,
        }
    }

    #[test]
    fn contended_snapshot_returns_unknown_and_never_spins() {
        let sample = AgeSample::default();
        sample.publish(Some(age(1)));
        sample.0.sequence.store(3, SeqCst);
        for _ in 0..100 {
            assert_eq!(sample.snapshot(), None);
        }
        sample.0.sequence.store(4, SeqCst);
        sample.publish(None);
        assert_eq!(sample.snapshot(), None);
    }

    #[test]
    fn sequence_exhaustion_stays_unknown_instead_of_wrapping_aba() {
        let sample = AgeSample::default();
        sample.0.sequence.store(u64::MAX - 1, SeqCst);
        sample.publish(Some(age(1)));
        assert_eq!(sample.snapshot(), None);
        sample.publish(Some(age(2)));
        assert_eq!(sample.snapshot(), None);
    }

    #[test]
    fn concurrent_age_and_delivery_never_mix_fields() {
        let age_sample = Arc::new(AgeSample::default());
        let delivery = Arc::new(DeliverySample::default());
        let barrier = Arc::new(Barrier::new(2));
        let a = age_sample.clone();
        let d = delivery.clone();
        let start = barrier.clone();
        let writer = std::thread::spawn(move || {
            start.wait();
            for n in 1..=20_000 {
                a.publish(Some(age(n)));
                d.publish(InputDeliverySnapshot {
                    packets: n,
                    max_packet_frames: n as u32,
                    max_callback_burst_frames: n,
                    gap_samples: n,
                    max_gap_100ns: n,
                    packet_bound_violations: n,
                    gap_bound_violations: n,
                    unavailable_clocks: n,
                    regressed_clocks: n,
                });
            }
        });
        barrier.wait();
        for _ in 0..30_000 {
            if let Some(sample) = age_sample.snapshot() {
                assert_eq!(sample, age(sample.stamp.generation));
            }
            if let Some(sample) = delivery.snapshot() {
                let n = sample.packets;
                assert_eq!(
                    sample,
                    InputDeliverySnapshot {
                        packets: n,
                        max_packet_frames: n as u32,
                        max_callback_burst_frames: n,
                        gap_samples: n,
                        max_gap_100ns: n,
                        packet_bound_violations: n,
                        gap_bound_violations: n,
                        unavailable_clocks: n,
                        regressed_clocks: n,
                    }
                );
            }
        }
        writer.join().unwrap();
        assert_eq!(age_sample.snapshot(), Some(age(20_000)));
        assert_eq!(delivery.snapshot().unwrap().packets, 20_000);
    }
}
