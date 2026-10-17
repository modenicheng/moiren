//! Relative queue budgets are admitted only after native timing negotiation.
use crate::latency::{
    LatencyBudgetError, LatencyBudgetPolicy, NativeStreamTiming, PreparedLatencyBudget,
    RenderQueuePolicy, prepare_latency_budget,
};
use serde::Serialize;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeLatencyProfile {
    #[default]
    Compatible,
    Balanced,
    Low,
}
impl NativeLatencyProfile {
    /// A graph producer writes bursts up to the master's admitted native
    /// target. Capture watermarks assume one capture period per packet and
    /// cannot be reused unchanged for an output follower with different periods.
    /// Leave normal callback phase room above the servo target, then reserve
    /// one complete producer burst beyond high water for bounded recovery.
    pub fn follower_input(
        self,
        master: NativeStreamTiming,
        follower: NativeStreamTiming,
        compatible: LatencyBudgetPolicy,
        master_queue: crate::latency::PreparedRenderQueue,
    ) -> Result<crate::latency::PreparedInputQueue, LatencyBudgetError> {
        if master_queue.capacity_frames != master.capacity_frames {
            return Err(LatencyBudgetError::InvalidRenderTarget);
        }
        master_queue.demand(0)?;
        let mut input = self.input_budget(master, follower, compatible)?.input;
        if self != Self::Compatible {
            let consumer_period = follower
                .period_frames
                .ok_or(LatencyBudgetError::MissingActualPeriod)?;
            input.high_water_frames = input
                .prime_frames
                .checked_add(master_queue.target_frames)
                .and_then(|frames| frames.checked_add(consumer_period))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            input.capacity_frames = input
                .high_water_frames
                .checked_add(master_queue.target_frames)
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        }
        Ok(input)
    }
    pub fn desired_period_ms(self) -> Option<f64> {
        match self {
            Self::Compatible => None,
            Self::Balanced => Some(10.0),
            Self::Low => Some(5.0),
        }
    }
    pub fn render_policy(
        self,
        timing: NativeStreamTiming,
        compatible: RenderQueuePolicy,
    ) -> Result<RenderQueuePolicy, LatencyBudgetError> {
        timing.validate()?;
        if self == Self::Compatible {
            return Ok(compatible);
        }
        let period = timing
            .period_frames
            .ok_or(LatencyBudgetError::MissingActualPeriod)?;
        let margin = period.div_ceil(if self == Self::Balanced { 2 } else { 4 });
        let frames = period
            .checked_add(margin)
            .and_then(|frames| frames.checked_add(margin))
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        let policy = RenderQueuePolicy::Target {
            frames,
            wake_margin_frames: margin,
            callback_budget_frames: margin,
        };
        crate::latency::prepare_render_queue(timing, policy)?;
        Ok(policy)
    }
    pub fn input_budget(
        self,
        input: NativeStreamTiming,
        output: NativeStreamTiming,
        compatible: LatencyBudgetPolicy,
    ) -> Result<PreparedLatencyBudget, LatencyBudgetError> {
        input.validate()?;
        output.validate()?;
        if self == Self::Compatible {
            return prepare_latency_budget(input, output, compatible);
        }
        let period = input
            .period_frames
            .ok_or(LatencyBudgetError::MissingActualPeriod)?;
        let render = self.render_policy(output, RenderQueuePolicy::FullCapacity)?;
        let target = crate::latency::prepare_render_queue(output, render)?.target_frames;
        let demand = u32::try_from(
            (u64::from(target) * u64::from(input.sample_rate))
                .div_ceil(u64::from(output.sample_rate)),
        )
        .map_err(|_| LatencyBudgetError::ArithmeticOverflow)?;
        let jitter = if self == Self::Balanced {
            period
        } else {
            period.div_ceil(2)
        };
        let target = demand
            .checked_add(period)
            .and_then(|frames| frames.checked_add(jitter))
            .and_then(|frames| frames.checked_add(2))
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        let capacity = period
            .checked_mul(2)
            .and_then(|frames| frames.checked_add(target))
            .and_then(|frames| frames.checked_add(1))
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        prepare_latency_budget(
            input,
            output,
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: capacity,
                input_target_frames: target,
                input_jitter_frames: jitter,
                render,
            },
        )
    }
}

pub(super) fn device_period(
    device: moiren_engine::node::DeviceOptions,
) -> Result<crate::shared_stream::SharedPeriodPolicy, moiren_engine::host::BackendError> {
    use moiren_engine::host::BackendError;
    device
        .validate()
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
    let desired_period_ms = match (device.period_frames, device.hardware_sample_rate) {
        (Some(frames), Some(rate)) => Some(frames as f64 * 1000.0 / f64::from(rate)),
        (Some(_), None) => {
            // Frame intent is in endpoint-rate units, not the graph's 48 kHz
            // units. Reject ambiguity before any native resources are probed.
            return Err(BackendError::Unsupported(
                "period_frames requires hardware_sample_rate; set both or omit period_frames"
                    .into(),
            ));
        }
        (None, _) => None,
    };
    let period = crate::shared_stream::SharedPeriodPolicy { desired_period_ms };
    period
        .validate()
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
    Ok(period)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_period_conversion_uses_explicit_rate_without_a_48k_assumption() {
        for (rate, frames) in [(44_100, 441), (48_000, 480), (96_000, 960)] {
            let period = device_period(moiren_engine::node::DeviceOptions {
                hardware_sample_rate: Some(rate),
                period_frames: Some(frames),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(period.desired_period_ms, Some(10.0));
            assert_eq!(
                crate::shared_stream::choose_shared_period(
                    crate::shared_stream::SharedPeriodCaps {
                        default_frames: frames as u32,
                        fundamental_frames: 1,
                        min_frames: 1,
                        max_frames: frames as u32 * 2,
                    },
                    rate,
                    period,
                ),
                Ok(frames as u32)
            );
        }
    }

    #[test]
    fn no_frame_intent_leaves_profile_or_endpoint_period_available() {
        for rate in [None, Some(44_100), Some(48_000), Some(96_000)] {
            let period = device_period(moiren_engine::node::DeviceOptions {
                hardware_sample_rate: rate,
                ..Default::default()
            })
            .unwrap();
            assert_eq!(period.desired_period_ms, None);
        }
        assert_eq!(NativeLatencyProfile::Compatible.desired_period_ms(), None);
        assert_eq!(
            NativeLatencyProfile::Balanced.desired_period_ms(),
            Some(10.0)
        );
        assert_eq!(NativeLatencyProfile::Low.desired_period_ms(), Some(5.0));
    }

    #[test]
    fn zero_rate_or_frame_period_is_rejected_before_conversion() {
        for (rate, frames) in [
            (Some(0), Some(441)),
            (Some(44_100), Some(0)),
            (Some(0), None),
        ] {
            assert!(
                device_period(moiren_engine::node::DeviceOptions {
                    hardware_sample_rate: rate,
                    period_frames: frames,
                    ..Default::default()
                })
                .is_err()
            );
        }
    }

    #[test]
    fn relative_profiles_use_actual_period_and_reject_impossible_capacity() {
        let timing = NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(240),
            capacity_frames: 960,
        };
        assert_eq!(
            crate::latency::prepare_render_queue(
                timing,
                NativeLatencyProfile::Balanced
                    .render_policy(timing, RenderQueuePolicy::FullCapacity)
                    .unwrap()
            )
            .unwrap()
            .target_frames,
            480
        );
        let low = NativeLatencyProfile::Low
            .input_budget(timing, timing, LatencyBudgetPolicy::Compatible)
            .unwrap();
        assert_eq!(low.render.target_frames, 360);
        assert_eq!(low.input.prime_frames, 722);
        assert!(
            NativeLatencyProfile::Low
                .render_policy(
                    NativeStreamTiming {
                        capacity_frames: 240,
                        ..timing
                    },
                    RenderQueuePolicy::FullCapacity
                )
                .is_err()
        );
        assert!(
            NativeLatencyProfile::Low
                .render_policy(
                    NativeStreamTiming {
                        period_frames: None,
                        ..timing
                    },
                    RenderQueuePolicy::FullCapacity
                )
                .is_err()
        );
    }
    #[test]
    fn relative_profiles_reject_invalid_or_overflowing_timing_without_panicking() {
        let valid = NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(240),
            capacity_frames: 960,
        };
        for invalid in [
            NativeStreamTiming {
                sample_rate: 0,
                ..valid
            },
            NativeStreamTiming {
                capacity_frames: 0,
                ..valid
            },
            NativeStreamTiming {
                period_frames: Some(0),
                ..valid
            },
        ] {
            for profile in [
                NativeLatencyProfile::Compatible,
                NativeLatencyProfile::Balanced,
                NativeLatencyProfile::Low,
            ] {
                assert_eq!(
                    profile.input_budget(invalid, valid, LatencyBudgetPolicy::Compatible),
                    Err(LatencyBudgetError::InvalidTiming)
                );
                assert_eq!(
                    profile.input_budget(valid, invalid, LatencyBudgetPolicy::Compatible),
                    Err(LatencyBudgetError::InvalidTiming)
                );
            }
        }
        assert_eq!(
            NativeLatencyProfile::Balanced.render_policy(
                NativeStreamTiming {
                    period_frames: Some(u32::MAX),
                    capacity_frames: u32::MAX,
                    ..valid
                },
                RenderQueuePolicy::FullCapacity
            ),
            Err(LatencyBudgetError::ArithmeticOverflow)
        );
    }
}
