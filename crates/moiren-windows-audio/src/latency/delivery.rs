//! Owner-local packet arrival observations. Arrival gaps are not sample age.
use super::{LatencyBudgetError, transport::InputDeliveryTiming};
use serde::Serialize;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputDeliverySnapshot {
    pub packets: u64,
    pub max_packet_frames: u32,
    pub max_callback_burst_frames: u64,
    pub gap_samples: u64,
    pub max_gap_100ns: u64,
    pub packet_bound_violations: u64,
    pub gap_bound_violations: u64,
    pub unavailable_clocks: u64,
    pub regressed_clocks: u64,
}

/// Prepared off RT; recording uses scalars only, with no storage growth.
pub struct InputDeliveryRecorder {
    snapshot: InputDeliverySnapshot,
    bound: Option<InputDeliveryTiming>,
    gap_limit_100ns: Option<u64>,
    last_delivery: Option<u64>,
}

impl InputDeliveryRecorder {
    pub fn new(bound: Option<InputDeliveryTiming>) -> Result<Self, LatencyBudgetError> {
        if bound.is_some_and(|d| {
            d.sample_rate == 0 || d.max_packet_frames == 0 || d.max_gap_frames == 0
        }) {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        Ok(Self {
            snapshot: InputDeliverySnapshot::default(),
            bound,
            gap_limit_100ns: bound.map(|d| {
                (u64::from(d.max_gap_frames) * 10_000_000).div_ceil(u64::from(d.sample_rate))
            }),
            last_delivery: None,
        })
    }

    pub fn packet(&mut self, frames: u32, arrival_qpc_100ns: Option<u64>) {
        if frames == 0 {
            return;
        }
        self.snapshot.packets = self.snapshot.packets.saturating_add(1);
        self.snapshot.max_packet_frames = self.snapshot.max_packet_frames.max(frames);
        if self.bound.is_some_and(|d| frames > d.max_packet_frames) {
            self.snapshot.packet_bound_violations =
                self.snapshot.packet_bound_violations.saturating_add(1);
        }
        if let Some(now) = arrival_qpc_100ns.filter(|&qpc| qpc > 0) {
            if let Some(previous) = self.last_delivery {
                if let Some(gap) = now.checked_sub(previous) {
                    self.snapshot.gap_samples = self.snapshot.gap_samples.saturating_add(1);
                    self.snapshot.max_gap_100ns = self.snapshot.max_gap_100ns.max(gap);
                    if self.gap_limit_100ns.is_some_and(|limit| gap > limit) {
                        self.snapshot.gap_bound_violations =
                            self.snapshot.gap_bound_violations.saturating_add(1);
                    }
                } else {
                    self.snapshot.regressed_clocks =
                        self.snapshot.regressed_clocks.saturating_add(1);
                }
            }
            self.last_delivery = Some(now);
        } else {
            self.last_delivery = None;
            self.snapshot.unavailable_clocks = self.snapshot.unavailable_clocks.saturating_add(1);
        }
    }

    pub fn callback_burst(&mut self, frames: u64) {
        self.snapshot.max_callback_burst_frames =
            self.snapshot.max_callback_burst_frames.max(frames);
    }

    pub fn snapshot(&self) -> InputDeliverySnapshot {
        self.snapshot
    }
}
