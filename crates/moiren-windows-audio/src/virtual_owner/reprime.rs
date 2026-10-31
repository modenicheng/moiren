//! Preparation-only microphone reset recovery margins, not measured latency.

use super::PreparedVirtualBudget;
use crate::latency::LatencyBudgetError;
use crate::latency::transport::TransportInputPolicy;
use serde::Serialize;

const TICKS_PER_SECOND: u128 = 10_000_000;

/// A reachable carrier threshold and nominal bounded wait after a hard reset.
/// Physical scheduling jitter can still exhaust the wait and require fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PreparedMicrophoneReprime {
    pub packet_frames: u32,
    pub required_frames: u32,
    pub max_wait_100ns: u64,
    /// Compatible storage cannot hold the ideal recovery threshold.
    pub capacity_limited: bool,
}

impl PreparedVirtualBudget {
    /// Bind a reset-only recovery margin to the actual native packet/carrier.
    /// This does not alter the prepared queue target or certify device timing.
    pub fn microphone_reprime(
        self,
        packet_frames: u32,
        capacity_frames: u32,
    ) -> Result<PreparedMicrophoneReprime, LatencyBudgetError> {
        self.requested.admit_native_packet(packet_frames)?;
        if capacity_frames < packet_frames
            || self.engine_sample_rate == 0
            || self.engine_block_frames == 0
            || self.engine_burst_frames < self.engine_block_frames
        {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        // The evidence DTO is public: never trust a caller-edited transfer,
        // FIR margin or queue watermark as an admitted recovery budget.
        let mut expected = self.requested.prepare_budget(
            self.engine_sample_rate,
            self.engine_block_frames,
            self.engine_burst_frames,
        )?;
        if let Some(negotiated) = self.native_packet_frames {
            if negotiated != packet_frames {
                return Err(LatencyBudgetError::InvalidDeliveryBound);
            }
            expected = expected.admit_native_packet(negotiated)?;
        }
        if self != expected {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        let (target, jitter) = match (self.requested.input, self.queue) {
            (TransportInputPolicy::Compatible, None) => (0, 0),
            (
                TransportInputPolicy::Target {
                    capacity_frames: requested_capacity,
                    target_frames,
                    jitter_frames,
                },
                Some(queue),
            ) if requested_capacity == capacity_frames
                && queue.capacity_frames == capacity_frames
                && queue.prime_frames == target_frames
                && target_frames > 0
                && target_frames < capacity_frames
                && self.wake_allowance_frames == Some(jitter_frames) =>
            {
                (target_frames, jitter_frames)
            }
            _ => return Err(LatencyBudgetError::InvalidInputTarget),
        };
        // Two packets can be consumed before the next large Engine block.
        // Reserve that complete publication gap too, without changing target.
        let transfer_margin = u64::from(self.native_transfer_frames)
            .checked_add(u64::from(packet_frames))
            .and_then(|frames| frames.checked_add(u64::from(self.fir_lookahead_frames)))
            .and_then(|frames| frames.checked_add(u64::from(jitter)))
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        let ideal_frames = u64::from(target)
            .max(u64::from(packet_frames) * 2)
            .max(transfer_margin);
        let capacity_limited = ideal_frames > u64::from(capacity_frames);
        if capacity_limited && matches!(self.requested.input, TransportInputPolicy::Target { .. }) {
            return Err(LatencyBudgetError::InsufficientInputBudget);
        }
        // Compatible storage can be smaller than the ideal recovery margin;
        // retain reachable bounded fallback and report that limitation.
        let required_frames = u32::try_from(ideal_frames.min(u64::from(capacity_frames)))
            .map_err(|_| LatencyBudgetError::ArithmeticOverflow)?;
        let native_rate = u128::from(crate::NATIVE_SAMPLE_RATE);
        // Cover rebuilding content plus FIR at the conservative production
        // floor (positive microphone correction slows publication), then a
        // complete Engine wake and the declared jitter. This is a nominal
        // recovery model; exhausted physical jitter still takes the fallback.
        let fill = ceil_ticks(
            u128::from(required_frames) + u128::from(self.fir_lookahead_frames),
            1000 * TICKS_PER_SECOND,
            native_rate * 998,
        )?;
        let wake = ceil_ticks(
            u128::from(self.engine_burst_frames),
            TICKS_PER_SECOND,
            u128::from(self.engine_sample_rate),
        )?;
        let jitter = ceil_ticks(u128::from(jitter), TICKS_PER_SECOND, native_rate)?;
        let modeled_wait = fill
            .checked_add(wake)
            .and_then(|ticks| ticks.checked_add(jitter))
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        let legacy_wait = ceil_ticks(
            u128::from(packet_frames),
            5 * TICKS_PER_SECOND,
            native_rate * 2,
        )?;
        Ok(PreparedMicrophoneReprime {
            packet_frames,
            required_frames,
            max_wait_100ns: modeled_wait.max(legacy_wait),
            capacity_limited,
        })
    }
}

fn ceil_ticks(frames: u128, scale: u128, rate: u128) -> Result<u64, LatencyBudgetError> {
    if rate == 0 {
        return Err(LatencyBudgetError::InvalidDeliveryBound);
    }
    let numerator = frames
        .checked_mul(scale)
        .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
    u64::try_from(numerator.div_ceil(rate)).map_err(|_| LatencyBudgetError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests;
