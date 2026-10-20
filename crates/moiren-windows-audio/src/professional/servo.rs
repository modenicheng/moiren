//! Occupancy-only follower control, shared by native and virtual FIR bridges.
use super::resample::CORRECTION_PPM_LIMIT;

const FOLLOWER_GAIN_PPM: f64 = 500.0;
const FOLLOWER_SMOOTHING: f64 = 0.01;

/// Pure arithmetic: no allocation, locking or clock reads on the audio path.
pub(crate) struct FollowerServo {
    target: usize,
    correction: f64,
}

impl FollowerServo {
    /// Two periods, floored at 64 frames and capped at half the ring capacity.
    pub(crate) fn start_target(period: usize, capacity: usize) -> usize {
        (period * 2).max(64).min(capacity / 2)
    }
    pub(crate) fn new(target: usize) -> Self {
        Self {
            target,
            correction: 0.0,
        }
    }
    pub(crate) fn target(&self) -> usize {
        self.target
    }
    /// Positive correction consumes source faster to reduce queue occupancy.
    pub(crate) fn observe(&mut self, queued: usize) -> f64 {
        let desired = ((queued as f64 - self.target as f64) / self.target as f64
            * FOLLOWER_GAIN_PPM)
            .clamp(-CORRECTION_PPM_LIMIT, CORRECTION_PPM_LIMIT);
        self.correction += (desired - self.correction) * FOLLOWER_SMOOTHING;
        self.correction
    }
}
