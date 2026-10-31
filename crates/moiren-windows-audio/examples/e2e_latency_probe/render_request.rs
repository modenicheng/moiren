//! Physical-output intent; it is not native negotiation or measured latency.
use anyhow::{Result, ensure};
#[cfg(any(windows, test))]
use moiren_engine::node::DeviceOptions;
use moiren_engine::{audio_config::BUFFER_SIZES, node::DeviceMode};
use moiren_windows_audio::latency::RenderQueuePolicy;
#[cfg(any(windows, test))]
use moiren_windows_audio::latency::shared_period::SharedInitializationApi;
#[cfg(any(windows, test))]
use moiren_windows_audio::latency::{NativeStreamTiming, shared_period::SharedPeriodReport};
use serde::Serialize;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct PhysicalRenderRequest {
    pub mode: Option<DeviceMode>,
    pub period_frames: Option<u32>,
}

impl PhysicalRenderRequest {
    pub fn validate(self, queue: RenderQueuePolicy) -> Result<()> {
        if let Some(period) = self.period_frames {
            ensure!(
                BUFFER_SIZES.contains(&period)
                    || (cfg!(feature = "vad-probe")
                        && period == 144
                        && self.mode == Some(DeviceMode::Exclusive)),
                "--render-period-frames must be a power of two in 64..=4096, or explicit Exclusive 144 with vad-probe"
            );
        }
        ensure!(
            self.mode != Some(DeviceMode::Exclusive)
                || matches!(queue, RenderQueuePolicy::FullCapacity),
            "Exclusive physical render requires --render-target-frames capacity"
        );
        Ok(())
    }

    #[cfg(any(windows, test))]
    pub fn apply(self, mut device: DeviceOptions) -> DeviceOptions {
        if let Some(mode) = self.mode {
            device.mode = mode;
        }
        if let Some(period) = self.period_frames {
            device.period_frames = Some(period as usize);
        }
        device
    }

    #[cfg(any(windows, test))]
    pub fn validate_observed(
        self,
        timing: NativeStreamTiming,
        shared: Option<SharedPeriodReport>,
    ) -> Result<()> {
        // Omission keeps the backend's existing default-period compatibility
        // policy. Explicit periods never silently accept that fallback.
        let Some(period) = self.period_frames else {
            return Ok(());
        };
        ensure!(
            timing.sample_rate > 0
                && timing.capacity_frames > 0
                && timing.period_frames == Some(period),
            "physical render period not accepted exactly: requested={period}, observed={timing:?}"
        );
        if self.mode.unwrap_or_default() == DeviceMode::Exclusive {
            ensure!(
                shared.is_none() && timing.capacity_frames == period,
                "Exclusive physical render must expose its exact native buffer without Shared observations"
            );
        } else {
            let shared = shared.ok_or_else(|| {
                anyhow::anyhow!("explicit Shared render period has no negotiation evidence")
            })?;
            ensure!(
                shared.initialization_api == Some(SharedInitializationApi::Client3)
                    && shared.fallback_reason.is_none()
                    && !shared.windows_auto_conversion
                    && shared.client3_failure.is_none()
                    && shared.period_selection_failure.is_none()
                    && shared
                        .actual_period_hresult
                        .is_none_or(|hresult| hresult >= 0)
                    && shared.requested_period_frames == Some(period)
                    && shared.actual_period_frames == Some(period),
                "explicit Shared render period was not accepted without fallback: {shared:?}"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::{
        audio_config::{AudioBackend, BUFFER_SIZES, SampleFormat},
        boundary::ClockRole,
    };
    use moiren_windows_audio::latency::shared_period::{
        SharedFallbackReason, SharedInitializationApi, SharedInitializationFailure,
        SharedInitializationStage, SharedPeriodCaps, SharedPeriodError,
        SharedPeriodSelectionFailure,
    };

    fn request(mode: DeviceMode) -> PhysicalRenderRequest {
        PhysicalRenderRequest {
            mode: Some(mode),
            period_frames: Some(128),
        }
    }

    fn timing() -> NativeStreamTiming {
        NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(128),
            capacity_frames: 128,
        }
    }

    fn shared() -> SharedPeriodReport {
        SharedPeriodReport {
            requested_period_frames: Some(128),
            actual_period_frames: Some(128),
            initialization_api: Some(SharedInitializationApi::Client3),
            ..Default::default()
        }
    }

    #[test]
    fn physical_render_apply_preserves_non_period_configuration_and_omitted_defaults() {
        let base = DeviceOptions {
            backend: AudioBackend::Wasapi,
            clock_role: ClockRole::Master,
            sample_format: SampleFormat::Float32,
            hardware_sample_rate: Some(48_000),
            ..Default::default()
        };
        assert_eq!(PhysicalRenderRequest::default().apply(base), base);
        let applied = request(DeviceMode::Exclusive).apply(base);
        assert_eq!(
            applied,
            DeviceOptions {
                mode: DeviceMode::Exclusive,
                period_frames: Some(128),
                ..base
            }
        );
        assert_eq!(
            PhysicalRenderRequest {
                mode: Some(DeviceMode::Exclusive),
                period_frames: None
            }
            .apply(base)
            .period_frames,
            base.period_frames
        );
    }

    #[test]
    fn physical_render_rejects_unsupported_periods_and_exclusive_target_queues() {
        for period in BUFFER_SIZES {
            assert!(
                PhysicalRenderRequest {
                    period_frames: Some(period),
                    ..Default::default()
                }
                .validate(RenderQueuePolicy::FullCapacity)
                .is_ok()
            );
        }
        for period in [0, 48, 63, 480, 8192, u32::MAX] {
            assert!(
                PhysicalRenderRequest {
                    period_frames: Some(period),
                    ..Default::default()
                }
                .validate(RenderQueuePolicy::FullCapacity)
                .is_err()
            );
        }
        let target = RenderQueuePolicy::Target {
            frames: 256,
            wake_margin_frames: 96,
            callback_budget_frames: 48,
        };
        assert!(request(DeviceMode::Exclusive).validate(target).is_err());
        assert!(request(DeviceMode::Shared).validate(target).is_ok());
    }

    #[test]
    fn physical_render_144_is_explicit_exclusive_only_and_never_accepts_alignment() {
        let request = PhysicalRenderRequest {
            mode: Some(DeviceMode::Exclusive),
            period_frames: Some(144),
        };
        assert_eq!(
            request.validate(RenderQueuePolicy::FullCapacity).is_ok(),
            cfg!(feature = "vad-probe")
        );
        for mode in [None, Some(DeviceMode::Shared)] {
            assert!(
                PhysicalRenderRequest { mode, ..request }
                    .validate(RenderQueuePolicy::FullCapacity)
                    .is_err()
            );
        }
        assert!(
            request
                .validate(RenderQueuePolicy::Target {
                    frames: 144,
                    wake_margin_frames: 96,
                    callback_budget_frames: 48,
                })
                .is_err()
        );
        let observed = NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(144),
            capacity_frames: 144,
        };
        assert!(request.validate_observed(observed, None).is_ok());
        for observed in [
            NativeStreamTiming {
                period_frames: Some(160),
                capacity_frames: 160,
                ..observed
            },
            NativeStreamTiming {
                capacity_frames: 256,
                ..observed
            },
            NativeStreamTiming {
                period_frames: None,
                ..observed
            },
        ] {
            assert!(request.validate_observed(observed, None).is_err());
        }
        for period in [143, 145, 160, 192, 480] {
            assert!(
                PhysicalRenderRequest {
                    period_frames: Some(period),
                    ..request
                }
                .validate(RenderQueuePolicy::FullCapacity)
                .is_err()
            );
        }
    }

    #[test]
    fn physical_render_requires_observed_exact_period_not_capacity_or_request_alone() {
        for mode in [DeviceMode::Shared, DeviceMode::Exclusive] {
            let shared = (mode == DeviceMode::Shared).then(shared);
            let request = request(mode);
            assert!(request.validate_observed(timing(), shared).is_ok());
            for observed in [
                NativeStreamTiming {
                    period_frames: None,
                    ..timing()
                },
                NativeStreamTiming {
                    period_frames: Some(256),
                    ..timing()
                },
                NativeStreamTiming {
                    sample_rate: 0,
                    ..timing()
                },
                NativeStreamTiming {
                    capacity_frames: 0,
                    ..timing()
                },
            ] {
                assert!(request.validate_observed(observed, shared).is_err());
            }
        }
        assert!(
            request(DeviceMode::Exclusive)
                .validate_observed(
                    NativeStreamTiming {
                        capacity_frames: 256,
                        ..timing()
                    },
                    None
                )
                .is_err()
        );
        assert!(
            request(DeviceMode::Exclusive)
                .validate_observed(timing(), Some(shared()))
                .is_err()
        );
    }

    #[test]
    fn physical_render_shared_explicit_period_refuses_compatibility_unknown_and_mismatch() {
        let request = request(DeviceMode::Shared);
        assert!(request.validate_observed(timing(), None).is_err());
        for observed in [
            SharedPeriodReport::default(),
            SharedPeriodReport {
                initialization_api: Some(SharedInitializationApi::LegacyClient),
                ..shared()
            },
            SharedPeriodReport {
                fallback_reason: Some(SharedFallbackReason::EnginePeriodLocked),
                ..shared()
            },
            SharedPeriodReport {
                windows_auto_conversion: true,
                ..shared()
            },
            SharedPeriodReport {
                actual_period_frames: None,
                ..shared()
            },
            SharedPeriodReport {
                actual_period_frames: Some(256),
                ..shared()
            },
            SharedPeriodReport {
                requested_period_frames: Some(256),
                ..shared()
            },
            SharedPeriodReport {
                actual_period_hresult: Some(-1),
                ..shared()
            },
            SharedPeriodReport {
                client3_failure: Some(SharedInitializationFailure {
                    stage: SharedInitializationStage::Capabilities,
                    hresult: -1,
                }),
                ..shared()
            },
            SharedPeriodReport {
                period_selection_failure: Some(SharedPeriodSelectionFailure {
                    error: SharedPeriodError::InvalidCapabilities,
                    sample_rate: 48_000,
                    desired_period_ms: None,
                    caps: SharedPeriodCaps {
                        default_frames: 480,
                        fundamental_frames: 48,
                        min_frames: 48,
                        max_frames: 960,
                    },
                }),
                ..shared()
            },
        ] {
            assert!(request.validate_observed(timing(), Some(observed)).is_err());
        }
    }

    #[test]
    fn physical_render_no_explicit_period_keeps_legacy_native_observations_unchanged() {
        let request = PhysicalRenderRequest::default();
        assert!(
            request
                .validate_observed(
                    NativeStreamTiming {
                        period_frames: None,
                        ..timing()
                    },
                    Some(SharedPeriodReport::default())
                )
                .is_ok()
        );
    }
}
