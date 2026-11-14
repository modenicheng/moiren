//! Period negotiation regressions kept separate from native packet ownership tests.
use super::*;

#[test]
fn shared_device_period_accepts_exact_non_power_of_two_requests_only_for_wasapi() {
    for frames in [64, 65, 144, 336, 441, 480, 4096] {
        for input in [false, true] {
            let device = DeviceOptions {
                period_frames: Some(frames),
                ..Default::default()
            };
            assert!(
                native_period_request_supported(device, input),
                "Shared {frames}"
            );
            if !frames.is_power_of_two() {
                assert!(
                    !native_period_request_supported(
                        DeviceOptions {
                            backend: moiren_engine::audio_config::AudioBackend::Asio,
                            ..device
                        },
                        input
                    ),
                    "ASIO {frames}"
                );
            }
        }
    }
    for frames in [0, 1, 63, 4097, usize::MAX] {
        assert!(!native_period_request_supported(
            DeviceOptions {
                period_frames: Some(frames),
                ..Default::default()
            },
            false
        ));
    }
}

#[test]
fn exact_shared_period_readback_refuses_mismatch_and_preserves_observed_frames() {
    let mut report = SharedPeriodReport {
        requested_period_frames: Some(336),
        initialization_api: Some(SharedInitializationApi::Client3),
        ..Default::default()
    };
    assert!(record_shared_period(480, 48_000, 48_000, &mut report).is_err());
    assert_eq!(report.engine_period_frames, Some(480));
    assert_eq!(report.actual_period_frames, Some(480));
    assert!(record_shared_period(336, 48_000, 48_000, &mut report).is_ok());
    report.requested_period_frames = None;
    assert!(record_shared_period(480, 48_000, 48_000, &mut report).is_ok());
}

#[test]
fn native_explicit_render_144_requires_exclusive_probe_and_not_capture_or_asio() {
    let device = DeviceOptions {
        mode: DeviceMode::Exclusive,
        period_frames: Some(144),
        ..Default::default()
    };
    assert_eq!(
        native_period_request_supported(device, false),
        cfg!(feature = "vad-probe")
    );
    assert!(!native_period_request_supported(device, true));
    assert!(native_period_request_supported(
        DeviceOptions {
            mode: DeviceMode::Shared,
            ..device
        },
        false
    ));
    assert!(!native_period_request_supported(
        DeviceOptions {
            backend: moiren_engine::audio_config::AudioBackend::Asio,
            ..device
        },
        false
    ));
    for period in [0, 143, 145, 160, 192, 480, 8192] {
        assert!(!native_period_request_supported(
            DeviceOptions {
                period_frames: Some(period),
                ..device
            },
            false
        ));
    }
    assert_eq!(
        exclusive_request_duration(
            &WasapiOptions {
                endpoint_id: "not-opened".into(),
                sample_rate: 48_000,
                device,
                input: false,
                exclusive_period_evidence: None,
            },
            100_000,
            30_000,
            48_000
        )
        .unwrap(),
        30_000
    );
}

fn minimum_period_evidence() -> ExclusivePeriodEvidenceHandle {
    Arc::new(std::sync::Mutex::new(ExclusivePeriodEvidence {
        qualification: "unqualified-request-only",
        requested_minimum: true,
        default_hns: None,
        minimum_hns: None,
        query_hresult: None,
        initial_request_hns: None,
        final_request_hns: None,
        alignment_retry: false,
        alignment_query_hresult: None,
        aligned_buffer_frames: None,
        first_initialize_hresult: None,
        retry_initialize_hresult: None,
        actual_buffer_frames: None,
        actual_buffer_hresult: None,
    }))
}

#[test]
fn exclusive_minimum_selection_preserves_default_and_refuses_invalid_query() {
    let mut options = WasapiOptions {
        endpoint_id: "not-opened".into(),
        sample_rate: 48_000,
        device: DeviceOptions {
            mode: DeviceMode::Exclusive,
            ..Default::default()
        },
        input: true,
        exclusive_period_evidence: None,
    };
    assert_eq!(
        exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
        100_000
    );
    options.device.period_frames = Some(256);
    assert_eq!(
        exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
        53_333
    );
    options.device.period_frames = None;
    let handle = minimum_period_evidence();
    options.exclusive_period_evidence = Some(handle.clone());
    assert_eq!(
        exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
        30_000
    );
    for invalid in [i64::MIN, 0, 999, 1_000_001] {
        handle.lock().unwrap().record_query(100_000, invalid);
        assert!(exclusive_request_duration(&options, 100_000, invalid, 48_000).is_err());
        let evidence = handle.lock().unwrap();
        assert_eq!(evidence.initial_request_hns, None);
        assert_eq!(evidence.final_request_hns, None);
        assert_eq!(evidence.first_initialize_hresult, None);
    }
}

#[test]
fn exclusive_minimum_refuses_shared_output_and_explicit_period_before_com() {
    for (input, mode, period, expected) in [
        (true, DeviceMode::Shared, None, "input Exclusive"),
        (false, DeviceMode::Exclusive, None, "input Exclusive"),
        (true, DeviceMode::Exclusive, Some(256), "explicit period"),
    ] {
        let handle = minimum_period_evidence();
        let options = WasapiOptions {
            endpoint_id: "not-opened".into(),
            sample_rate: 48_000,
            device: DeviceOptions {
                mode,
                period_frames: period,
                ..Default::default()
            },
            input,
            exclusive_period_evidence: Some(handle.clone()),
        };
        match WasapiStream::open_inner(options) {
            Err(error) => assert!(error.to_string().contains(expected)),
            Ok(_) => panic!("invalid request must refuse before native activation"),
        }
        let evidence = handle.lock().unwrap();
        assert_eq!(evidence.query_hresult, None);
        assert_eq!(evidence.first_initialize_hresult, None);
    }
}

#[test]
fn exclusive_minimum_receipt_is_omitted_by_default_and_preserves_partial_failure() {
    use crate::{
        SessionDuration,
        capture::{CaptureReport, CaptureSource},
    };
    let mut report = CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped);
    let default = serde_json::to_value(&report).unwrap();
    assert!(default.get("exclusive_period").is_none());
    assert_eq!(default["schema_version"], 12);
    let handle = minimum_period_evidence();
    handle.lock().unwrap().record_query_hresult(-1);
    report.exclusive_period = Some(*handle.lock().unwrap());
    report.failure = Some("preserved preparation failure".into());
    let partial = serde_json::to_value(&report).unwrap();
    assert_eq!(partial["failure"], "preserved preparation failure");
    assert_eq!(partial["exclusive_period"]["query_hresult"], -1);
    assert!(partial["exclusive_period"]["minimum_hns"].is_null());
    assert!(partial["exclusive_period"]["initial_request_hns"].is_null());
}
