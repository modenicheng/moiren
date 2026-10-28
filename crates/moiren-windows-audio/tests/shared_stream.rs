use moiren_windows_audio::{
    SessionDuration,
    capture::{CaptureError, CaptureOptions},
    latency::TelemetryMode,
    latency::shared_period::{
        SharedFallbackReason, SharedInitializationApi, SharedInitializationFailure,
        SharedInitializationStage, SharedPeriodCaps, SharedPeriodError, SharedPeriodPolicy,
        SharedPeriodReport, SharedPeriodSelectionFailure, choose_shared_period,
    },
};

fn caps() -> SharedPeriodCaps {
    SharedPeriodCaps {
        default_frames: 480,
        fundamental_frames: 48,
        min_frames: 96,
        max_frames: 960,
    }
}

#[test]
fn rounds_to_legal_multiples_with_device_bounds_and_native_rate() {
    for (rate, milliseconds, expected) in [
        (48_000, Some(1.0), 96),
        (48_000, Some(5.0), 240),
        (48_000, Some(21.0), 960),
        (44_100, Some(2.1), 96),
        (44_100, Some(2.2), 144),
        (44_100, None, 480),
        (u32::MAX, Some(10.0), 960),
    ] {
        assert_eq!(
            choose_shared_period(
                caps(),
                rate,
                SharedPeriodPolicy {
                    desired_period_ms: milliseconds
                }
            ),
            Ok(expected)
        );
    }
    let unaligned_bounds = SharedPeriodCaps {
        min_frames: 97,
        max_frames: 959,
        ..caps()
    };
    for (milliseconds, expected) in [(0.1, 144), (5.0, 240), (50.0, 912)] {
        assert_eq!(
            choose_shared_period(
                unaligned_bounds,
                48_000,
                SharedPeriodPolicy {
                    desired_period_ms: Some(milliseconds)
                }
            ),
            Ok(expected)
        );
    }
}

#[test]
fn accepts_vm_capabilities_with_an_unaligned_default_period() {
    let vm_caps = SharedPeriodCaps {
        default_frames: 488,
        fundamental_frames: 35,
        min_frames: 139,
        max_frames: 488,
    };

    assert_eq!(
        choose_shared_period(vm_caps, 48_000, SharedPeriodPolicy::default()),
        Ok(245)
    );
    assert_eq!(
        choose_shared_period(
            vm_caps,
            48_000,
            SharedPeriodPolicy {
                desired_period_ms: None,
            }
        ),
        Ok(488)
    );
}

#[test]
fn wide_rounding_does_not_overflow_near_u32_limit() {
    let caps = SharedPeriodCaps {
        default_frames: u32::MAX,
        fundamental_frames: u32::MAX,
        min_frames: 1,
        max_frames: u32::MAX,
    };
    assert_eq!(
        choose_shared_period(
            caps,
            u32::MAX,
            SharedPeriodPolicy {
                desired_period_ms: Some(1000.0)
            }
        ),
        Ok(u32::MAX)
    );
}

#[test]
fn rejects_bad_capabilities_and_conversion_overflow() {
    for invalid in [
        SharedPeriodCaps {
            fundamental_frames: 0,
            ..caps()
        },
        SharedPeriodCaps {
            min_frames: 0,
            ..caps()
        },
        SharedPeriodCaps {
            min_frames: 961,
            ..caps()
        },
        SharedPeriodCaps {
            default_frames: 961,
            ..caps()
        },
        SharedPeriodCaps {
            max_frames: 479,
            ..caps()
        },
    ] {
        assert_eq!(
            choose_shared_period(invalid, 48_000, SharedPeriodPolicy::default()),
            Err(SharedPeriodError::InvalidCapabilities)
        );
    }
    assert_eq!(
        choose_shared_period(caps(), 0, SharedPeriodPolicy::default()),
        Err(SharedPeriodError::InvalidSampleRate)
    );
    assert_eq!(
        choose_shared_period(
            caps(),
            48_000,
            SharedPeriodPolicy {
                desired_period_ms: Some(f64::MAX)
            }
        ),
        Err(SharedPeriodError::DesiredPeriodOverflow)
    );
}

#[test]
fn invalid_policy_is_rejected_before_capture_preparation() {
    for milliseconds in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let shared_period = SharedPeriodPolicy {
            desired_period_ms: Some(milliseconds),
        };
        let capture = CaptureOptions {
            artifact: None,
            endpoint_id: "fake".into(),
            duration: SessionDuration::UntilStopped,
            latency_telemetry: TelemetryMode::Disabled,
            shared_period,
        };
        assert_eq!(
            capture.validate(),
            Err(CaptureError::SharedPeriod(
                SharedPeriodError::InvalidDesiredPeriod
            ))
        );
    }
}

#[test]
fn serializes_actual_period_and_explicit_fallback_without_invented_measurements() {
    let success = SharedPeriodReport {
        desired_period_ms: Some(5.0),
        requested_period_frames: Some(240),
        actual_period_frames: Some(336),
        caps: Some(caps()),
        initialization_api: Some(SharedInitializationApi::Client3),
        ..Default::default()
    };
    let json = serde_json::to_value(success).unwrap();
    assert_eq!(json["requested_period_frames"], 240);
    assert_eq!(json["actual_period_frames"], 336);
    assert_eq!(json["initialization_api"], "client3");
    assert!(json["fallback_reason"].is_null());
    let fallback = SharedPeriodReport {
        initialization_api: Some(SharedInitializationApi::LegacyClient),
        fallback_reason: Some(SharedFallbackReason::Client3Unavailable),
        client3_failure: Some(SharedInitializationFailure {
            stage: SharedInitializationStage::InterfaceQuery,
            hresult: -1,
        }),
        ..Default::default()
    };
    let json = serde_json::to_value(fallback).unwrap();
    assert_eq!(json["initialization_api"], "legacy_client");
    assert_eq!(json["fallback_reason"], "client3_unavailable");
    assert_eq!(json["client3_failure"]["stage"], "interface_query");
    assert!(json["requested_period_frames"].is_null());
    assert!(json["actual_period_frames"].is_null());
    let json = serde_json::to_value(SharedPeriodReport::default()).unwrap();
    assert!(json["initialization_api"].is_null());
}

#[test]
fn serializes_period_selection_failure_with_raw_request_and_capabilities() {
    let failure = SharedPeriodSelectionFailure {
        error: SharedPeriodError::InvalidCapabilities,
        sample_rate: 48_000,
        desired_period_ms: Some(5.0),
        caps: SharedPeriodCaps {
            default_frames: 480,
            fundamental_frames: 0,
            min_frames: 96,
            max_frames: 480,
        },
    };
    let report = SharedPeriodReport {
        caps: Some(failure.caps),
        period_selection_failure: Some(failure),
        ..Default::default()
    };
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(
        json["period_selection_failure"]["error"],
        "invalid_capabilities"
    );
    assert_eq!(json["period_selection_failure"]["sample_rate"], 48_000);
    assert_eq!(json["period_selection_failure"]["desired_period_ms"], 5.0);
    assert_eq!(
        json["period_selection_failure"]["caps"]["fundamental_frames"],
        0
    );
    assert_eq!(json["caps"]["max_frames"], 480);
}
