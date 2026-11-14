//! Service delay from a packet's host/driver stamp, never source sample age.
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::sync::atomic::{
    AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketStampOrigin {
    #[default]
    Unknown,
    DriverTransfer,
    HostSubmission,
}

/// Independently sampled counters, not a transactional live summary. Read
/// after the unique completion writer joins for final min/max/count evidence.
/// A service delay says nothing about oldest audio, PCM correctness or output.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketServiceSnapshot {
    pub origin: PacketStampOrigin,
    pub qpc_frequency_hz: Option<u64>,
    pub observations: u64,
    pub unavailable_clocks: u64,
    pub future_stamps: u64,
    pub conversion_overflows: u64,
    /// Floor of the smallest observed raw-tick delta in 100 ns units.
    pub min_delay_100ns: Option<u64>,
    /// Ceil of the largest observed raw-tick delta in 100 ns units.
    pub max_delay_100ns: Option<u64>,
}

/// One completion writer, many control readers. Record calls must be made by
/// that single writer; fixed load/store updates avoid CAS retry loops in audio
/// service. Shared storage does not permit two writers to combine summaries.
#[cfg(any(windows, test))]
pub(crate) struct PacketServiceRecorder {
    origin: PacketStampOrigin,
    frequency: Option<u64>,
    observations: AtomicU64,
    unavailable: AtomicU64,
    future: AtomicU64,
    overflows: AtomicU64,
    minimum: AtomicU64,
    maximum: AtomicU64,
}

#[cfg(any(windows, test))]
impl PacketServiceRecorder {
    pub(crate) fn new(origin: PacketStampOrigin, frequency: Option<u64>) -> Self {
        Self {
            origin,
            frequency: frequency.filter(|&n| n != 0),
            observations: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            future: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            minimum: AtomicU64::new(u64::MAX),
            maximum: AtomicU64::new(0),
        }
    }

    /// Compare the SAME raw QPC domain before conversion. Subtracting two
    /// independently rounded absolute times loses the conservative bounds.
    pub(crate) fn record(&self, stamp_ticks: u64, collected_ticks: Option<u64>) {
        let Some((frequency, collected)) = self
            .frequency
            .zip(collected_ticks.filter(|&n| n != 0))
            .filter(|_| stamp_ticks != 0 && self.origin != PacketStampOrigin::Unknown)
        else {
            increment(&self.unavailable);
            return;
        };
        let Some(delta) = collected.checked_sub(stamp_ticks) else {
            increment(&self.future);
            return;
        };
        let numerator = u128::from(delta) * 10_000_000;
        let frequency = u128::from(frequency);
        let (Ok(floor), Ok(ceil)) = (
            u64::try_from(numerator / frequency),
            u64::try_from(numerator.div_ceil(frequency)),
        ) else {
            increment(&self.overflows);
            return;
        };
        self.minimum
            .store(self.minimum.load(Relaxed).min(floor), Relaxed);
        self.maximum
            .store(self.maximum.load(Relaxed).max(ceil), Relaxed);
        self.observations
            .store(self.observations.load(Relaxed).saturating_add(1), Release);
    }

    pub(crate) fn snapshot(&self) -> PacketServiceSnapshot {
        let observations = self.observations.load(Acquire);
        PacketServiceSnapshot {
            origin: self.origin,
            qpc_frequency_hz: self.frequency,
            observations,
            unavailable_clocks: self.unavailable.load(Relaxed),
            future_stamps: self.future.load(Relaxed),
            conversion_overflows: self.overflows.load(Relaxed),
            min_delay_100ns: (observations != 0).then(|| self.minimum.load(Relaxed)),
            max_delay_100ns: (observations != 0).then(|| self.maximum.load(Relaxed)),
        }
    }
}

#[cfg(any(windows, test))]
fn increment(counter: &AtomicU64) {
    counter.store(counter.load(Relaxed).saturating_add(1), Relaxed);
}

#[cfg(test)]
#[path = "packet_service/tests.rs"]
mod tests;
