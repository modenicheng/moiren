//! Approximate scalar snapshots, never a coherent audio/metadata transaction.
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{
        AtomicBool, AtomicU64, AtomicUsize,
        Ordering::{Acquire, Relaxed},
    },
};
use std::time::Instant;

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct BridgeSnapshot {
    pub captured_frames: u64,
    /// Publication cadence, measured on capture rather than on render demand.
    /// Startup maxima cover the first sixteen nonempty packets only.
    pub packets: u64,
    pub first_packet_frames: usize,
    pub max_packet_frames: usize,
    pub max_packet_interval_us: u64,
    pub max_startup_packet_interval_us: u64,
    /// Number of packets already published before the longest startup gap.
    pub max_startup_gap_after_packet: u64,
    pub max_startup_gap_at_output_frame: u64,
    pub max_startup_gap_qpc_interval_us: u64,
    pub first_underrun_captured_frames: u64,
    pub first_underrun_packets: u64,
    /// Approximate publication age at the first shortfall, using a monotonic
    /// preparation epoch shared by the two bridge owners.
    pub first_underrun_packet_age_us: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    pub nonfinite_samples: u64,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub last_device_position_frames: u64,
    pub last_qpc_100ns: u64,
    pub output_frames: u64,
    pub priming_frames: u64,
    pub prime_discarded_frames: u64,
    pub underrun_frames: u64,
    /// Shortfalls observed before the capture owner advertised retirement.
    /// Cumulative: a later finite tail cannot erase an earlier live shortfall.
    pub live_underrun_frames: u64,
    pub last_underrun_at_output_frame: u64,
    pub last_underrun_producer_finished: bool,
    pub resets: u64,
    pub fill_frames: usize,
    pub min_fill_frames: usize,
    pub max_fill_frames: usize,
    pub correction_ppm: f64,
}
pub(super) struct Counters {
    pub captured_frames: AtomicU64,
    pub packets: AtomicU64,
    pub first_packet_frames: AtomicUsize,
    pub max_packet_frames: AtomicUsize,
    pub max_packet_interval_us: AtomicU64,
    pub max_startup_packet_interval_us: AtomicU64,
    pub max_startup_gap_after_packet: AtomicU64,
    pub max_startup_gap_at_output_frame: AtomicU64,
    pub max_startup_gap_qpc_interval_us: AtomicU64,
    pub first_underrun_captured_frames: AtomicU64,
    pub first_underrun_packets: AtomicU64,
    pub first_underrun_packet_age_us: AtomicU64,
    pub last_packet_elapsed_us: AtomicU64,
    pub prepared_at: Instant,
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
    pub underrun_frames: AtomicU64,
    pub live_underrun_frames: AtomicU64,
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
    pub fn new(capacity: usize) -> Self {
        Self {
            captured_frames: AtomicU64::new(0),
            packets: AtomicU64::new(0),
            first_packet_frames: AtomicUsize::new(0),
            max_packet_frames: AtomicUsize::new(0),
            max_packet_interval_us: AtomicU64::new(0),
            max_startup_packet_interval_us: AtomicU64::new(0),
            max_startup_gap_after_packet: AtomicU64::new(0),
            max_startup_gap_at_output_frame: AtomicU64::new(0),
            max_startup_gap_qpc_interval_us: AtomicU64::new(0),
            first_underrun_captured_frames: AtomicU64::new(0),
            first_underrun_packets: AtomicU64::new(0),
            first_underrun_packet_age_us: AtomicU64::new(0),
            last_packet_elapsed_us: AtomicU64::new(0),
            prepared_at: Instant::now(),
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
            underrun_frames: AtomicU64::new(0),
            live_underrun_frames: AtomicU64::new(0),
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
    pub fn producer_finished(&self) -> bool {
        self.0.producer_finished.load(Acquire)
    }
    pub fn snapshot(&self) -> BridgeSnapshot {
        let c = &self.0;
        BridgeSnapshot {
            captured_frames: c.captured_frames.load(Relaxed),
            packets: c.packets.load(Relaxed),
            first_packet_frames: c.first_packet_frames.load(Relaxed),
            max_packet_frames: c.max_packet_frames.load(Relaxed),
            max_packet_interval_us: c.max_packet_interval_us.load(Relaxed),
            max_startup_packet_interval_us: c.max_startup_packet_interval_us.load(Relaxed),
            max_startup_gap_after_packet: c.max_startup_gap_after_packet.load(Relaxed),
            max_startup_gap_at_output_frame: c.max_startup_gap_at_output_frame.load(Relaxed),
            max_startup_gap_qpc_interval_us: c.max_startup_gap_qpc_interval_us.load(Relaxed),
            first_underrun_captured_frames: c.first_underrun_captured_frames.load(Relaxed),
            first_underrun_packets: c.first_underrun_packets.load(Relaxed),
            first_underrun_packet_age_us: c.first_underrun_packet_age_us.load(Relaxed),
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
            underrun_frames: c.underrun_frames.load(Relaxed),
            live_underrun_frames: c.live_underrun_frames.load(Relaxed),
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
