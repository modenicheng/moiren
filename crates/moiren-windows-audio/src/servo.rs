//! Occupancy-only follower control, shared by native and virtual FIR bridges.
use crate::resample::CORRECTION_PPM_LIMIT;

/// Gain per unit of relative occupancy error. Carrier geometry bounds
/// |(queued - target) / target| to about one at the empty/full rails, so the
/// gain must equal the correction limit for full authority to be reachable
/// at all: with a 500 ppm gain the ±2000 ppm clamp was mathematically
/// unreachable and effective authority stayed at ±500 ppm — far below the
/// sustained clock deficits it had to correct.
const FOLLOWER_GAIN_PPM: f64 = CORRECTION_PPM_LIMIT;
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
    /// Current smoothed correction in ppm. Diagnostics and probes only; the
    /// control-side water-level probe lands with the soak-evidence change.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn correction(&self) -> f64 {
        self.correction
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Carrier geometry bounds |relative error| to about one at the empty and
    /// full rails, so the ±2000 ppm clamp is only reachable when the gain
    /// itself is 2000 ppm per unit error. The old 500 ppm gain capped real
    /// authority at ±500 ppm — the audit's proven authority defect.
    #[test]
    fn correction_authority_reaches_the_clamp_at_the_ring_rails() {
        let target = 2_255_usize;
        let mut servo = FollowerServo::new(target);
        for _ in 0..10_000 {
            let correction = servo.observe(0);
            assert!((-CORRECTION_PPM_LIMIT..=0.0).contains(&correction));
        }
        assert!((servo.observe(0) + CORRECTION_PPM_LIMIT).abs() < 1.0);
        let mut servo = FollowerServo::new(target);
        for _ in 0..10_000 {
            let correction = servo.observe(2 * target);
            assert!((0.0..=CORRECTION_PPM_LIMIT).contains(&correction));
        }
        assert!((servo.observe(2 * target) - CORRECTION_PPM_LIMIT).abs() < 1.0);
    }

    #[test]
    fn sign_convention_matches_queue_occupancy() {
        let mut servo = FollowerServo::new(1_000);
        assert!(servo.observe(1_200) > 0.0, "above target consumes faster");
        let mut servo = FollowerServo::new(1_000);
        assert!(servo.observe(800) < 0.0, "below target consumes slower");
    }

    #[test]
    fn correction_is_clamped_beyond_the_rails() {
        let mut servo = FollowerServo::new(100);
        for _ in 0..10_000 {
            assert!(servo.observe(1_000_000) <= CORRECTION_PPM_LIMIT);
        }
        // The one-pole smoother converges asymptotically; floating-point
        // absorption stalls it within ~1e-12 of the limit, never exactly on it.
        assert!((servo.observe(1_000_000) - CORRECTION_PPM_LIMIT).abs() < 1e-6);
    }

    #[test]
    fn on_target_holds_zero_correction() {
        let mut servo = FollowerServo::new(1_000);
        assert_eq!(servo.observe(1_000), 0.0);
        assert_eq!(servo.observe(1_000), 0.0);
    }

    #[test]
    fn start_target_bounds() {
        assert_eq!(FollowerServo::start_target(256, 4_800), 512);
        assert_eq!(FollowerServo::start_target(8, 4_800), 64);
        assert_eq!(FollowerServo::start_target(4_800, 4_800), 2_400);
    }
}
