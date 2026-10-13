use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorInitializationApi {
    LegacyClient,
    Client3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorPeriodFallback {
    Client3Unavailable,
    CapabilityQueryUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorStreamDiagnostics {
    pub sample_rate: u32,
    /// Unknown native queries remain unknown, independently of the request.
    pub actual_period_frames: Option<u32>,
    pub capacity_frames: u32,
    pub requested_period_frames: Option<u32>,
    pub initialization_api: Option<MonitorInitializationApi>,
    pub period_fallback: Option<MonitorPeriodFallback>,
}

impl MonitorStreamDiagnostics {
    pub fn period_duration(self) -> Option<Duration> {
        self.actual_period_frames
            .filter(|frames| *frames != 0)
            .and_then(|frames| self.frames_duration(u64::from(frames)))
    }

    /// A queue or period scale, rounded upward; no end-to-end measurement.
    pub fn frames_duration(self, frames: u64) -> Option<Duration> {
        if self.sample_rate == 0 {
            return None;
        }
        let nanos = frames
            .checked_mul(1_000_000_000)?
            .div_ceil(u64::from(self.sample_rate));
        Some(Duration::from_nanos(nanos))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorInputBudget {
    pub capacity_frames: u32,
    pub prime_frames: u32,
    pub servo_reference_frames: u32,
    pub low_water_frames: u32,
    pub high_water_frames: u32,
    pub modeled_minimum_frames: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRenderBudget {
    pub capacity_frames: u32,
    pub target_frames: u32,
    pub deadline_budget_frames: Option<u32>,
}

/// Fixed native preparation results; no COM object crosses this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorPreparedDiagnostics {
    pub input: MonitorStreamDiagnostics,
    pub output: MonitorStreamDiagnostics,
    pub input_queue: MonitorInputBudget,
    pub render_queue: MonitorRenderBudget,
}

/// Independent scalar reads; this is not a coherent audio transaction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorObservation {
    pub prepared: MonitorPreparedDiagnostics,
    pub processed_frames: u64,
    /// Approximate Bridge inventory; this is not capture age or physical latency.
    pub input_fill_frames: u64,
    pub correction_ppm: f64,
}

impl MonitorObservation {
    pub fn input_fill_duration(self) -> Option<Duration> {
        self.prepared.input.frames_duration(self.input_fill_frames)
    }
}

#[cfg(windows)]
impl MonitorPreparedDiagnostics {
    pub(super) fn from_native(
        budget: moiren_windows_audio::latency::PreparedLatencyBudget,
        input_period: moiren_windows_audio::shared_stream::SharedPeriodReport,
        output_period: moiren_windows_audio::shared_stream::SharedPeriodReport,
    ) -> Self {
        use moiren_windows_audio::shared_stream::{SharedFallbackReason, SharedInitializationApi};
        let stream =
            |timing: moiren_windows_audio::latency::NativeStreamTiming,
             period: moiren_windows_audio::shared_stream::SharedPeriodReport| {
                MonitorStreamDiagnostics {
                    sample_rate: timing.sample_rate,
                    actual_period_frames: timing.period_frames,
                    capacity_frames: timing.capacity_frames,
                    requested_period_frames: period.requested_period_frames,
                    initialization_api: period.initialization_api.map(|api| match api {
                        SharedInitializationApi::LegacyClient => {
                            MonitorInitializationApi::LegacyClient
                        }
                        SharedInitializationApi::Client3 => MonitorInitializationApi::Client3,
                    }),
                    period_fallback: period.fallback_reason.map(|reason| match reason {
                        SharedFallbackReason::Client3Unavailable => {
                            MonitorPeriodFallback::Client3Unavailable
                        }
                        SharedFallbackReason::CapabilityQueryUnsupported => {
                            MonitorPeriodFallback::CapabilityQueryUnsupported
                        }
                    }),
                }
            };
        Self {
            input: stream(budget.input_timing, input_period),
            output: stream(budget.output_timing, output_period),
            input_queue: MonitorInputBudget {
                capacity_frames: budget.input.capacity_frames,
                prime_frames: budget.input.prime_frames,
                servo_reference_frames: budget.input.servo_reference_frames,
                low_water_frames: budget.input.low_water_frames,
                high_water_frames: budget.input.high_water_frames,
                modeled_minimum_frames: budget.input.modeled_minimum_frames,
            },
            render_queue: MonitorRenderBudget {
                capacity_frames: budget.render.capacity_frames,
                target_frames: budget.render.target_frames,
                deadline_budget_frames: budget.render.deadline_budget_frames,
            },
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use moiren_windows_audio::{
        latency::{LatencyBudgetPolicy, NativeStreamTiming, prepare_latency_budget},
        shared_stream::{SharedFallbackReason, SharedInitializationApi, SharedPeriodReport},
    };

    #[test]
    fn native_diagnostics_preserve_budgets_unknown_period_and_initialization_fallback() {
        let budget = prepare_latency_budget(
            NativeStreamTiming {
                sample_rate: 44_100,
                period_frames: None,
                capacity_frames: 512,
            },
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: Some(288),
                capacity_frames: 1056,
            },
            LatencyBudgetPolicy::Compatible,
        )
        .unwrap();
        let input_period = SharedPeriodReport {
            requested_period_frames: Some(220),
            initialization_api: Some(SharedInitializationApi::LegacyClient),
            fallback_reason: Some(SharedFallbackReason::Client3Unavailable),
            ..SharedPeriodReport::default()
        };
        let output_period = SharedPeriodReport {
            requested_period_frames: Some(240),
            actual_period_frames: Some(288),
            initialization_api: Some(SharedInitializationApi::Client3),
            ..SharedPeriodReport::default()
        };
        let diagnostics =
            MonitorPreparedDiagnostics::from_native(budget, input_period, output_period);
        assert_eq!(diagnostics.input.sample_rate, 44_100);
        assert_eq!(diagnostics.input.capacity_frames, 512);
        assert_eq!(diagnostics.input.actual_period_frames, None);
        assert_eq!(diagnostics.input.requested_period_frames, Some(220));
        assert_eq!(
            diagnostics.input.initialization_api,
            Some(MonitorInitializationApi::LegacyClient)
        );
        assert_eq!(
            diagnostics.input.period_fallback,
            Some(MonitorPeriodFallback::Client3Unavailable)
        );
        assert_eq!(diagnostics.output.actual_period_frames, Some(288));
        assert_eq!(diagnostics.output.requested_period_frames, Some(240));
        assert_eq!(
            diagnostics.output.initialization_api,
            Some(MonitorInitializationApi::Client3)
        );
        assert_eq!(diagnostics.output.period_fallback, None);
        assert_eq!(
            diagnostics.input_queue,
            MonitorInputBudget {
                capacity_frames: 8192,
                prime_frames: 2048,
                servo_reference_frames: 2048,
                low_water_frames: 2,
                high_water_frames: 8191,
                modeled_minimum_frames: None,
            }
        );
        assert_eq!(
            diagnostics.render_queue,
            MonitorRenderBudget {
                capacity_frames: 1056,
                target_frames: 1056,
                deadline_budget_frames: None,
            }
        );
        let unsupported = MonitorPreparedDiagnostics::from_native(
            budget,
            SharedPeriodReport {
                fallback_reason: Some(SharedFallbackReason::CapabilityQueryUnsupported),
                ..input_period
            },
            output_period,
        );
        assert_eq!(
            unsupported.input.period_fallback,
            Some(MonitorPeriodFallback::CapabilityQueryUnsupported)
        );
    }
}
