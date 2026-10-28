//! Approximate counters plus independently coherent optional delivery/age DTOs.
//! The complete snapshot is not a cross-owner audio/metadata transaction.
mod live;
#[cfg(test)]
mod tests;
use super::QueuedFrameAge;
use crate::latency::InputDeliverySnapshot;
use live::{AgeSample, DeliverySample};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
};

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct BridgeSnapshot {
    /// Latest completed native packet/callback publication. None before native
    /// publication (including graph followers), or during a concurrent write.
    pub native_delivery: Option<InputDeliverySnapshot>,
    /// Oldest pending qualified frame sampled before a nonempty read's queue
    /// recovery/priming/transfer. Resets invalidate this sample immediately.
    /// This diagnostic estimate is not physical end-to-end latency.
    pub oldest_frame_age: Option<QueuedFrameAge>,
    pub captured_frames: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    pub nonfinite_samples: u64,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub last_device_position_frames: u64,
    /// Latest received valid packet stamp, including packets dropped by overflow.
    /// This does not measure the oldest queued audio.
    pub last_qpc_100ns: u64,
    pub output_frames: u64,
    pub priming_frames: u64,
    pub prime_discarded_frames: u64,
    /// Intentional old-frame removal while interpolation is already running.
    pub backlog_discarded_frames: u64,
    pub backlog_recoveries: u64,
    /// Old queued frames removed because the injected age estimate exceeded
    /// the live policy, distinct from overflow and high-water recovery.
    pub age_discarded_frames: u64,
    pub age_recoveries: u64,
    /// Successful output frames rendered with the bounded low-water correction.
    pub low_water_guard_frames: u64,
    pub underrun_frames: u64,
    pub last_underrun_at_output_frame: u64,
    pub last_underrun_producer_finished: bool,
    pub resets: u64,
    pub fill_frames: usize,
    pub min_fill_frames: usize,
    pub max_fill_frames: usize,
    pub correction_ppm: f64,
}
pub(super) struct Counters {
    pub queue_events: crate::latency::queue::QueueHistoryObserver,
    pub delivery_events: crate::latency::events::EventHistoryObserver,
    pub native_delivery: DeliverySample,
    pub oldest_frame_age: AgeSample,
    pub captured_frames: AtomicU64,
    pub written_frames: AtomicU64,
    pub dropped_frames: AtomicU64,
    pub silent_frames: AtomicU64,
    pub nonfinite_samples: AtomicU64,
    pub discontinuities: AtomicU64,
    pub timestamp_errors: AtomicU64,
    pub last_device_position_frames: AtomicU64,
    pub last_qpc_100ns: AtomicU64,
    pub output_frames: AtomicU64,
    pub priming_frames: AtomicU64,
    pub prime_discarded_frames: AtomicU64,
    pub backlog_discarded_frames: AtomicU64,
    pub backlog_recoveries: AtomicU64,
    pub age_discarded_frames: AtomicU64,
    pub age_recoveries: AtomicU64,
    pub low_water_guard_frames: AtomicU64,
    pub underrun_frames: AtomicU64,
    pub last_underrun_at_output_frame: AtomicU64,
    pub last_underrun_producer_finished: AtomicBool,
    pub resets: AtomicU64,
    pub fill_frames: AtomicUsize,
    pub min_fill_frames: AtomicUsize,
    pub max_fill_frames: AtomicUsize,
    pub correction_bits: AtomicU64,
    pub producer_finished: AtomicBool,
}
impl Counters {
    pub fn new(
        capacity: usize,
        delivery_events: crate::latency::events::EventHistoryObserver,
        queue_events: crate::latency::queue::QueueHistoryObserver,
    ) -> Self {
        Self {
            queue_events,
            delivery_events,
            native_delivery: DeliverySample::default(),
            oldest_frame_age: AgeSample::default(),
            captured_frames: AtomicU64::new(0),
            written_frames: AtomicU64::new(0),
            dropped_frames: AtomicU64::new(0),
            silent_frames: AtomicU64::new(0),
            nonfinite_samples: AtomicU64::new(0),
            discontinuities: AtomicU64::new(0),
            timestamp_errors: AtomicU64::new(0),
            last_device_position_frames: AtomicU64::new(0),
            last_qpc_100ns: AtomicU64::new(0),
            output_frames: AtomicU64::new(0),
            priming_frames: AtomicU64::new(0),
            prime_discarded_frames: AtomicU64::new(0),
            backlog_discarded_frames: AtomicU64::new(0),
            backlog_recoveries: AtomicU64::new(0),
            age_discarded_frames: AtomicU64::new(0),
            age_recoveries: AtomicU64::new(0),
            low_water_guard_frames: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            last_underrun_at_output_frame: AtomicU64::new(0),
            last_underrun_producer_finished: AtomicBool::new(false),
            resets: AtomicU64::new(0),
            fill_frames: AtomicUsize::new(0),
            min_fill_frames: AtomicUsize::new(capacity),
            max_fill_frames: AtomicUsize::new(0),
            correction_bits: AtomicU64::new(0.0f64.to_bits()),
            producer_finished: AtomicBool::new(false),
        }
    }
}
#[derive(Clone)]
pub struct BridgeObserver(pub(super) Arc<Counters>);
impl BridgeObserver {
    /// Consumer-owned causal inventory boundaries, not a timestamped replay of
    /// producer publications or a source sample-age estimate.
    pub fn queue_history(&self) -> Option<crate::latency::queue::QueueHistorySnapshot> {
        self.0.queue_events.snapshot()
    }
    /// Control-side bounded native callback history, separate from the
    /// approximate aggregate snapshot. No graph read manufactures arrivals.
    pub fn delivery_events(&self) -> Option<crate::latency::events::EventHistorySnapshot> {
        self.0.delivery_events.snapshot()
    }
    pub fn producer_finished(&self) -> bool {
        self.0.producer_finished.load(Relaxed)
    }
    pub fn snapshot(&self) -> BridgeSnapshot {
        self.0.snapshot()
    }
}
impl Counters {
    pub(super) fn snapshot(&self) -> BridgeSnapshot {
        let c = self;
        BridgeSnapshot {
            native_delivery: c.native_delivery.snapshot(),
            oldest_frame_age: c.oldest_frame_age.snapshot(),
            captured_frames: c.captured_frames.load(Relaxed),
            written_frames: c.written_frames.load(Relaxed),
            dropped_frames: c.dropped_frames.load(Relaxed),
            silent_frames: c.silent_frames.load(Relaxed),
            nonfinite_samples: c.nonfinite_samples.load(Relaxed),
            discontinuities: c.discontinuities.load(Relaxed),
            timestamp_errors: c.timestamp_errors.load(Relaxed),
            last_device_position_frames: c.last_device_position_frames.load(Relaxed),
            last_qpc_100ns: c.last_qpc_100ns.load(Relaxed),
            output_frames: c.output_frames.load(Relaxed),
            priming_frames: c.priming_frames.load(Relaxed),
            prime_discarded_frames: c.prime_discarded_frames.load(Relaxed),
            backlog_discarded_frames: c.backlog_discarded_frames.load(Relaxed),
            backlog_recoveries: c.backlog_recoveries.load(Relaxed),
            age_discarded_frames: c.age_discarded_frames.load(Relaxed),
            age_recoveries: c.age_recoveries.load(Relaxed),
            low_water_guard_frames: c.low_water_guard_frames.load(Relaxed),
            underrun_frames: c.underrun_frames.load(Relaxed),
            last_underrun_at_output_frame: c.last_underrun_at_output_frame.load(Relaxed),
            last_underrun_producer_finished: c.last_underrun_producer_finished.load(Relaxed),
            resets: c.resets.load(Relaxed),
            fill_frames: c.fill_frames.load(Relaxed),
            min_fill_frames: c.min_fill_frames.load(Relaxed),
            max_fill_frames: c.max_fill_frames.load(Relaxed),
            correction_ppm: f64::from_bits(c.correction_bits.load(Relaxed)),
        }
    }
}
