// Compile the new module without changing the shared latency.rs export file.
use moiren_windows_audio::latency;
#[path = "../src/latency/qualification.rs"]
mod qualification;

use latency::{MarkerAnalysisConfig, MarkerCorrelation, MarkerGenerator, analyze_marker};
use qualification::*;

fn format(sample_rate: u32) -> EvidenceFormat {
    EvidenceFormat {
        sample_rate,
        channels: 2,
        sample_format: "f32".into(),
    }
}

fn configuration() -> EvidenceConfiguration {
    EvidenceConfiguration {
        source_kind: EvidenceSourceKind::ProcessLoopback,
        source_identity: "process-identity".into(),
        source_details: None,
        output_endpoint_id: "physical-output".into(),
        processing_format: format(48_000),
        native_input_format: format(48_000),
        native_output_format: format(48_000),
        measurement_format: format(96_000),
        graph_id: "input-bus-output".into(),
        signal_id: "marker-v1".into(),
        controlled_load_id: "ordinary-cpu-load-v1".into(),
    }
}

fn expected() -> QualificationExpectation {
    QualificationExpectation {
        source_revision: "revision-current".into(),
        binary_hash: "binary-current".into(),
        current_config_hash: "configuration-current".into(),
        target_machine_id: "target-machine".into(),
        windows_build: "windows-build".into(),
        calibration_hash: "calibration-current".into(),
        evaluated_at_unix_seconds: 900_000,
        configuration: configuration(),
    }
}

fn detections() -> Vec<MarkerCorrelation> {
    // These are detector outputs from real synthetic PCM, not assigned delays.
    let mut generator = MarkerGenerator::new(255, 0.1).unwrap();
    let mut reference = vec![0.0; 255];
    generator.fill(&mut reference);
    [480, 576, 672, 768, 864]
        .into_iter()
        .map(|delay| {
            let mut recording = vec![0.0; 255 + 960];
            recording[delay..delay + 255].copy_from_slice(&reference);
            analyze_marker(
                &reference,
                &recording,
                MarkerAnalysisConfig {
                    max_marker_frames: 255,
                    max_recording_frames: 1215,
                    max_delay_frames: 960,
                    max_correlation_samples: 255 * 961,
                    min_correlation: 0.9,
                    min_peak_margin: 0.15,
                    min_rms: 0.001,
                },
            )
            .unwrap()
        })
        .collect()
}

fn report(run_id: &str, duration: f64) -> QualificationReport {
    QualificationReport {
        schema_version: QUALIFICATION_SCHEMA_VERSION,
        identity: EvidenceRunIdentity {
            run_id: run_id.into(),
            source_revision: "revision-current".into(),
            binary_hash: "binary-current".into(),
            config_hash: "configuration-current".into(),
            machine_id: "target-machine".into(),
            windows_build: "windows-build".into(),
        },
        expected_config_hash: "configuration-current".into(),
        configuration: configuration(),
        boundary: EvidenceBoundary::PhysicalCable,
        calibration: Some(CalibrationProvenance {
            reference_id: "synthetic-test-calibration".into(),
            evidence_id: "synthetic-test-calibration-artifact".into(),
            output_endpoint_id: "physical-output".into(),
            measurement_format: format(96_000),
            common_sample_clock: true,
            uncertainty_ms: 0.1,
            valid_until_unix_seconds: 1_000_000,
            calibration_hash: "calibration-current".into(),
        }),
        input_queue: QueueEvidence::default(),
        output_queue: QueueEvidence::default(),
        observed_duration_seconds: duration,
        observed_measurement_frames: (duration * 96_000.0) as u64,
        waveform_evidence_id: "synthetic-test-waveform".into(),
        steady_state: true,
        controlled_load: true,
        expected_marker_count: 5,
        distribution: Some(marker_distribution(&detections(), 96_000, 5).unwrap()),
        quality: QualificationQuality::default(),
    }
}

fn evaluate(report: QualificationReport) -> Result<(), QualificationError> {
    evaluate_qualification(
        &[report],
        &expected(),
        QualificationLevel::Experimental30Minutes,
        8,
    )
}

#[test]
fn percentiles_are_nearest_ranks_of_actual_marker_detections_at_the_explicit_rate() {
    let detections = detections();
    let result = marker_distribution(&detections, 96_000, 5).unwrap();
    assert_eq!(result.marker_count, 5);
    assert_eq!(result.sample_rate, 96_000);
    assert_eq!(result.p50_ms, 7.0);
    assert_eq!(result.p95_ms, 9.0);
    assert_eq!(result.p99_ms, 9.0);
    assert_eq!(result.max_ms, 9.0);
    let at_48k = marker_distribution(&detections, 48_000, 5).unwrap();
    assert_eq!(at_48k.p50_ms, 14.0);
    assert_eq!(at_48k.p95_ms, 18.0);
}

#[test]
fn percentiles_use_the_entire_unsorted_population_and_include_the_worst_tail() {
    let actual = detections();
    let mut population = vec![actual[0]; 100];
    population[0] = actual[4];
    for item in &mut population[1..6] {
        *item = actual[3];
    }
    let result = marker_distribution(&population, 96_000, 100).unwrap();
    assert_eq!(result.p50_ms, 5.0);
    assert_eq!(result.p95_ms, 8.0);
    assert_eq!(result.p99_ms, 8.0);
    assert_eq!(result.max_ms, 9.0);
}

#[test]
fn distribution_rejects_empty_inputs_zero_rates_limits_and_invalid_detections() {
    let actual = detections();
    assert_eq!(
        marker_distribution(&[], 48_000, 10),
        Err(QualificationError::MissingMarkers)
    );
    assert_eq!(
        marker_distribution(&actual, 0, 5),
        Err(QualificationError::InvalidSampleRate)
    );
    for limit in [0, 4] {
        assert_eq!(
            marker_distribution(&actual, 96_000, limit),
            Err(QualificationError::LimitExceeded)
        );
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 1.01] {
        let mut invalid = actual.clone();
        invalid[0].confidence = value;
        assert_eq!(
            marker_distribution(&invalid, 96_000, 5),
            Err(QualificationError::InvalidDetection)
        );
        invalid = actual.clone();
        invalid[0].peak_margin = value;
        assert_eq!(
            marker_distribution(&invalid, 96_000, 5),
            Err(QualificationError::InvalidDetection)
        );
    }
    let mut invalid = actual;
    invalid[0].peak_margin = 1.0;
    invalid[0].confidence = 0.5;
    assert_eq!(
        marker_distribution(&invalid, 96_000, 5),
        Err(QualificationError::InvalidDetection)
    );
}

#[test]
fn calibrated_thirty_minute_runs_qualify_experimentally_for_each_source_kind() {
    for source_kind in [EvidenceSourceKind::Vad, EvidenceSourceKind::ProcessLoopback] {
        let mut run = report("one", 1800.0);
        let mut expectation = expected();
        run.configuration.source_kind = source_kind;
        expectation.configuration.source_kind = source_kind;
        assert_eq!(
            evaluate_qualification(
                &[run],
                &expectation,
                QualificationLevel::Experimental30Minutes,
                1
            ),
            Ok(())
        );
    }
}

#[test]
fn two_distinct_two_hour_runs_on_the_target_machine_qualify_a_default() {
    assert_eq!(
        evaluate_qualification(
            &[report("one", 7200.0), report("two", 7200.0)],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            2,
        ),
        Ok(())
    );
}

#[test]
fn a_long_single_run_or_duplicate_run_ids_cannot_claim_repetition() {
    assert_eq!(
        evaluate_qualification(
            &[report("one", 14_400.0)],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            2
        ),
        Err(QualificationError::InsufficientRepetitions)
    );
    assert_eq!(
        evaluate_qualification(
            &[report("one", 7200.0), report("one", 7200.0)],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            2
        ),
        Err(QualificationError::DuplicateRun)
    );
}

#[test]
fn short_nonfinite_and_inconsistent_observation_durations_refuse_qualification() {
    for duration in [0.0, -1.0, 1799.999, f64::NAN, f64::INFINITY] {
        assert!(evaluate(report("one", duration)).is_err());
    }
    assert_eq!(
        evaluate_qualification(
            &[report("one", 7199.0), report("two", 7200.0)],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            2
        ),
        Err(QualificationError::InsufficientDuration)
    );
    let mut run = report("one", 1800.0);
    run.observed_measurement_frames = 96_000;
    assert_eq!(evaluate(run), Err(QualificationError::InvalidDuration));
}

#[test]
fn one_frame_short_of_the_pcm_gate_cannot_be_rounded_into_a_pass() {
    let mut run = report("one", 1800.0);
    run.observed_measurement_frames -= 1;
    assert_eq!(evaluate(run), Err(QualificationError::InsufficientDuration));
    let mut short = report("two", 7200.0);
    short.observed_measurement_frames -= 1;
    assert_eq!(
        evaluate_qualification(
            &[report("one", 7200.0), short],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            2,
        ),
        Err(QualificationError::InsufficientDuration)
    );
}

#[test]
fn internal_or_uncalibrated_evidence_cannot_qualify_physical_e2e() {
    for boundary in [EvidenceBoundary::Internal, EvidenceBoundary::WasapiLoopback] {
        let mut run = report("one", 1800.0);
        run.boundary = boundary;
        assert_eq!(evaluate(run), Err(QualificationError::InternalOnly));
    }
    let mut run = report("one", 1800.0);
    run.calibration = None;
    assert_eq!(evaluate(run), Err(QualificationError::Uncalibrated));
    for mutation in 0..4 {
        let mut run = report("one", 1800.0);
        let calibration = run.calibration.as_mut().unwrap();
        match mutation {
            0 => calibration.reference_id.clear(),
            1 => calibration.evidence_id.clear(),
            2 => calibration.common_sample_clock = false,
            _ => calibration.output_endpoint_id = "different-output".into(),
        }
        assert_eq!(evaluate(run), Err(QualificationError::Uncalibrated));
    }
    let mut run = report("one", 1800.0);
    run.boundary = EvidenceBoundary::ExternalDualChannel;
    assert_eq!(evaluate(run), Ok(()));
}

#[test]
fn missing_or_uncounted_markers_are_never_a_zero_latency_success() {
    let mut run = report("one", 1800.0);
    run.distribution = None;
    assert_eq!(evaluate(run), Err(QualificationError::MissingMarkers));
    for count in [0, 4, 6] {
        let mut run = report("one", 1800.0);
        run.distribution.as_mut().unwrap().marker_count = count;
        assert_eq!(evaluate(run), Err(QualificationError::MissingMarkers));
    }
    let mut run = report("one", 1800.0);
    run.expected_marker_count = 0;
    assert_eq!(evaluate(run), Err(QualificationError::MissingMarkers));
}

#[test]
fn every_quality_counter_including_missing_markers_and_hidden_recovery_is_a_refusal() {
    for counter in 0..7 {
        let mut run = report("one", 1800.0);
        match counter {
            0 => run.quality.dropped_frames = 1,
            1 => run.quality.silent_frames = 1,
            2 => run.quality.underrun_frames = 1,
            3 => run.quality.discontinuities = 1,
            4 => run.quality.epoch_resets = 1,
            5 => run.quality.marker_missing = 1,
            _ => run.quality.sample_gap_frames = 1,
        }
        assert_eq!(evaluate(run), Err(QualificationError::QualityFailure));
    }
}

#[test]
fn delivery_bound_violations_are_hard_failures_even_with_a_passing_preflight_and_distribution() {
    let mut value = serde_json::to_value(report("one", 1800.0)).unwrap();
    value["quality"]["delivery_bound_violations"] = serde_json::json!(1);
    let run: QualificationReport = serde_json::from_value(value).unwrap();
    assert_eq!(evaluate(run), Err(QualificationError::QualityFailure));
}

#[test]
fn calibration_expiry_hash_and_uncertainty_cannot_be_ignored() {
    for mutation in 0..4 {
        let mut value = serde_json::to_value(report("one", 1800.0)).unwrap();
        value["calibration"]["valid_until_unix_seconds"] = serde_json::json!(1_000_000);
        value["calibration"]["calibration_hash"] = serde_json::json!("calibration-current");
        value["calibration"]["uncertainty_ms"] = serde_json::json!(0.1);
        match mutation {
            0 => value["calibration"]["valid_until_unix_seconds"] = serde_json::json!(899_999),
            1 => value["calibration"]["valid_until_unix_seconds"] = serde_json::json!(900_000),
            2 => value["calibration"]["calibration_hash"] = serde_json::json!("old-calibration"),
            _ => value["calibration"]["uncertainty_ms"] = serde_json::json!(-0.1),
        }
        let mut expectation_value = serde_json::to_value(expected()).unwrap();
        expectation_value["evaluated_at_unix_seconds"] = serde_json::json!(900_000);
        expectation_value["calibration_hash"] = serde_json::json!("calibration-current");
        let run: QualificationReport = serde_json::from_value(value).unwrap();
        let expectation: QualificationExpectation =
            serde_json::from_value(expectation_value).unwrap();
        assert_eq!(
            evaluate_qualification(
                &[run],
                &expectation,
                QualificationLevel::Experimental30Minutes,
                1,
            ),
            Err(QualificationError::Uncalibrated)
        );
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut run = report("one", 1800.0);
        run.calibration.as_mut().unwrap().uncertainty_ms = invalid;
        assert_eq!(evaluate(run), Err(QualificationError::Uncalibrated));
    }
}

#[test]
fn calibration_uncertainty_consumes_latency_headroom_without_relaxing_the_target() {
    let mut value = serde_json::to_value(report("one", 1800.0)).unwrap();
    value["calibration"]["uncertainty_ms"] = serde_json::json!(1.1);
    let run: QualificationReport = serde_json::from_value(value).unwrap();
    assert_eq!(evaluate(run), Err(QualificationError::LatencyTargetMissed));
}

#[test]
fn startup_uncontrolled_load_and_missing_waveform_artifacts_are_not_steady_state_evidence() {
    for mutation in 0..3 {
        let mut run = report("one", 1800.0);
        match mutation {
            0 => run.steady_state = false,
            1 => run.controlled_load = false,
            _ => run.waveform_evidence_id.clear(),
        }
        assert_eq!(evaluate(run), Err(QualificationError::InvalidMetadata));
    }
}

#[test]
fn recorded_and_current_expected_hashes_and_binary_identity_must_all_match() {
    for mutation in 0..4 {
        let mut run = report("one", 1800.0);
        match mutation {
            0 => run.identity.config_hash = "stale".into(),
            1 => run.expected_config_hash = "stale".into(),
            2 => {
                run.identity.config_hash = "stale".into();
                run.expected_config_hash = "stale".into();
            }
            _ => run.identity.binary_hash = "stale".into(),
        }
        assert_eq!(evaluate(run), Err(QualificationError::StaleIdentity));
    }
    let mut run = report("one", 1800.0);
    run.identity.source_revision = "old-source".into();
    assert_eq!(evaluate(run), Err(QualificationError::StaleIdentity));
}

#[test]
fn endpoint_source_machine_windows_build_and_all_formats_must_match_expectations() {
    for mutation in 0..12 {
        let mut run = report("one", 1800.0);
        match mutation {
            0 => run.configuration.output_endpoint_id = "other".into(),
            1 => run.configuration.source_kind = EvidenceSourceKind::Vad,
            2 => run.configuration.processing_format.sample_rate = 44_100,
            3 => run.configuration.native_input_format.channels = 1,
            4 => run.configuration.native_output_format.sample_format = "i16".into(),
            5 => run.configuration.measurement_format.sample_rate = 48_000,
            6 => run.identity.machine_id = "other-machine".into(),
            7 => run.identity.windows_build = "other-build".into(),
            8 => run.configuration.graph_id = "other-graph".into(),
            9 => run.configuration.signal_id = "other-signal".into(),
            10 => run.configuration.controlled_load_id = "other-load".into(),
            _ => run.configuration.source_identity = "other-process".into(),
        }
        assert_eq!(
            evaluate(run),
            Err(QualificationError::MismatchedConfiguration)
        );
    }
    let mut run = report("one", 1800.0);
    run.calibration
        .as_mut()
        .unwrap()
        .measurement_format
        .sample_rate = 48_000;
    assert_eq!(evaluate(run), Err(QualificationError::Uncalibrated));
    let mut run = report("one", 1800.0);
    run.distribution.as_mut().unwrap().sample_rate = 48_000;
    assert_eq!(evaluate(run), Err(QualificationError::InvalidDistribution));
}

#[test]
fn nonfinite_negative_or_nonmonotonic_percentiles_are_refused() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
        for percentile in 0..4 {
            let mut run = report("one", 1800.0);
            let distribution = run.distribution.as_mut().unwrap();
            match percentile {
                0 => distribution.p50_ms = invalid,
                1 => distribution.p95_ms = invalid,
                2 => distribution.p99_ms = invalid,
                _ => distribution.max_ms = invalid,
            }
            assert_eq!(evaluate(run), Err(QualificationError::InvalidDistribution));
        }
    }
    for mutation in 0..3 {
        let mut run = report("one", 1800.0);
        let distribution = run.distribution.as_mut().unwrap();
        match mutation {
            0 => distribution.p50_ms = 9.5,
            1 => distribution.p99_ms = 8.5,
            _ => distribution.max_ms = 8.5,
        }
        assert_eq!(evaluate(run), Err(QualificationError::InvalidDistribution));
    }
}

#[test]
fn p95_target_is_inclusive_and_p50_eight_ms_is_only_an_exploration_target() {
    let mut run = report("one", 1800.0);
    run.calibration.as_mut().unwrap().uncertainty_ms = 0.0;
    let distribution = run.distribution.as_mut().unwrap();
    distribution.p50_ms = 9.0;
    distribution.p95_ms = 10.0;
    distribution.p99_ms = 15.0;
    distribution.max_ms = 20.0;
    assert_eq!(evaluate(run.clone()), Ok(()));
    run.distribution.as_mut().unwrap().p95_ms = 10.0001;
    assert_eq!(evaluate(run), Err(QualificationError::LatencyTargetMissed));
}

#[test]
fn queue_settings_cannot_replace_measurements_or_imply_latency() {
    let mut run = report("one", 1800.0);
    run.input_queue = QueueEvidence {
        requested_capacity_frames: Some(8192),
        requested_target_frames: Some(256),
        effective_capacity_frames: Some(8192),
        effective_target_frames: Some(1024),
        actual_period_frames: None,
    };
    run.output_queue = QueueEvidence {
        requested_capacity_frames: Some(1024),
        requested_target_frames: Some(128),
        effective_capacity_frames: Some(1024),
        effective_target_frames: Some(256),
        actual_period_frames: Some(128),
    };
    assert_eq!(evaluate(run.clone()), Ok(()));
    run.distribution = None;
    assert_eq!(evaluate(run), Err(QualificationError::MissingMarkers));
}

#[test]
fn one_failed_window_refuses_the_entire_submitted_run_set() {
    let mut failed = report("failed", 7200.0);
    failed.quality.epoch_resets = 1;
    assert_eq!(
        evaluate_qualification(
            &[report("one", 7200.0), failed, report("two", 7200.0)],
            &expected(),
            QualificationLevel::DefaultTwoHourRepeated,
            3
        ),
        Err(QualificationError::QualityFailure)
    );
}

#[test]
fn empty_sets_run_caps_and_unsupported_schemas_are_refused() {
    assert_eq!(
        evaluate_qualification(
            &[],
            &expected(),
            QualificationLevel::Experimental30Minutes,
            8
        ),
        Err(QualificationError::InsufficientRepetitions)
    );
    for limit in [0, 1] {
        assert_eq!(
            evaluate_qualification(
                &[report("one", 7200.0), report("two", 7200.0)],
                &expected(),
                QualificationLevel::DefaultTwoHourRepeated,
                limit
            ),
            Err(QualificationError::LimitExceeded)
        );
    }
    let mut run = report("one", 1800.0);
    run.schema_version += 1;
    assert_eq!(evaluate(run), Err(QualificationError::UnsupportedSchema));
}

#[test]
fn empty_expectations_invalid_formats_and_oversized_provenance_are_refused() {
    let mut expectation = expected();
    expectation.current_config_hash.clear();
    assert_eq!(
        evaluate_qualification(
            &[report("one", 1800.0)],
            &expectation,
            QualificationLevel::Experimental30Minutes,
            1
        ),
        Err(QualificationError::InvalidMetadata)
    );
    let mut run = report("one", 1800.0);
    run.identity.run_id = "x".repeat(4097);
    assert_eq!(evaluate(run), Err(QualificationError::InvalidMetadata));
    let mut run = report("one", 1800.0);
    let mut expectation = expected();
    run.configuration.measurement_format.channels = 0;
    expectation.configuration.measurement_format.channels = 0;
    assert_eq!(
        evaluate_qualification(
            &[run],
            &expectation,
            QualificationLevel::Experimental30Minutes,
            1
        ),
        Err(QualificationError::InvalidMetadata)
    );
}

#[test]
fn versioned_reports_round_trip_without_a_serialized_success_claim() {
    let run = report("one", 1800.0);
    let value = serde_json::to_value(&run).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["boundary"], "physical_cable");
    assert_eq!(value["configuration"]["source_kind"], "process_loopback");
    assert_eq!(
        value["input_queue"]["actual_period_frames"],
        serde_json::Value::Null
    );
    assert!(value.get("qualified").is_none());
    assert_eq!(value["quality"]["delivery_bound_violations"], 0);
    let restored: QualificationReport = serde_json::from_value(value).unwrap();
    assert_eq!(restored, run);
    assert_eq!(evaluate(restored), Ok(()));
    let mut legacy_value = serde_json::to_value(&run).unwrap();
    legacy_value["quality"]
        .as_object_mut()
        .unwrap()
        .remove("delivery_bound_violations");
    let restored: QualificationReport = serde_json::from_value(legacy_value).unwrap();
    assert_eq!(restored.quality.delivery_bound_violations, 0);
}
