use super::*;

#[test]
fn process_collection_control_unwind_keeps_owned_partial_state_available() {
    let mut partial = Vec::new();
    let result = guarded_run(&mut partial, |value| {
        value.push("prepared recorder");
        panic!("control failure");
    });
    assert!(result.is_err());
    assert_eq!(partial, ["prepared recorder"]);
}

#[test]
fn process_collection_earlier_recorder_deadline_prevents_later_source_poll() {
    let mut polls = 0;
    let result = after_recorder(true, || {
        polls += 1;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(polls, 0);
}

fn phases() -> Phases {
    Phases {
        ceiling: Duration::from_secs(20),
        start_requested: Duration::ZERO,
        preroll: Duration::from_millis(100),
        postroll: Duration::from_millis(100),
        source_duration: Duration::from_secs(1),
        ready: None,
        activated: None,
        running: None,
        submitted: None,
    }
}

#[test]
fn process_collection_requires_both_native_starts_and_preroll_not_timeline_advance() {
    let mut phases = phases();
    assert_eq!(
        phases.tick(Duration::ZERO, Health::default()).unwrap(),
        Action::Wait
    );
    let health = Health {
        graph_started: true,
        recorder_running: true,
        ..Health::default()
    };
    assert_eq!(
        phases.tick(Duration::from_millis(10), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_millis(109), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_millis(110), health).unwrap(),
        Action::ActivateSource
    );
    assert_eq!(
        phases.tick(Duration::from_millis(111), health).unwrap(),
        Action::Wait
    );
}

#[test]
fn process_collection_earlier_recorder_deadline_and_setup_time_never_renew_budget() {
    let mut phases = phases();
    assert!(
        phases
            .tick(
                Duration::from_secs(19),
                Health {
                    recorder_deadline: true,
                    ..Health::default()
                }
            )
            .is_err()
    );
    assert!(
        phases
            .tick(Duration::from_secs(20), Health::default())
            .is_err()
    );
    let mut delayed = super::tests::phases();
    delayed.activated = Some(Duration::from_secs(1));
    assert!(
        delayed
            .tick(
                Duration::from_secs(7),
                Health {
                    graph_started: true,
                    recorder_running: true,
                    ..Health::default()
                }
            )
            .is_err()
    );
}

#[test]
fn process_collection_postroll_requires_observed_running_and_submission() {
    let mut phases = phases();
    phases.activated = Some(Duration::ZERO);
    assert!(
        phases
            .tick(
                Duration::from_secs(1),
                Health {
                    source_submitted: true,
                    ..Health::default()
                }
            )
            .is_err()
    );
    let health = Health {
        graph_started: true,
        recorder_running: true,
        source_running: true,
        source_submitted: true,
        ..Health::default()
    };
    assert_eq!(
        phases.tick(Duration::from_secs(1), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_millis(1100), health).unwrap(),
        Action::Complete
    );
}

#[test]
fn process_collection_first_running_ack_after_delayed_return_cannot_bypass_deadline() {
    for millis in [5000, 6000] {
        let mut phases = phases();
        phases.activated = Some(Duration::ZERO);
        let result = phases.tick(
            Duration::from_millis(millis),
            Health {
                graph_started: true,
                recorder_running: true,
                source_running: true,
                ..Default::default()
            },
        );
        assert!(
            result.is_err(),
            "first Running observed at {millis}ms bypassed 5s deadline"
        );
    }
}

#[test]
fn process_collection_first_submitted_ack_after_delayed_return_cannot_bypass_deadline() {
    for (millis, postroll) in [(8000, 100), (9000, 100), (8000, 0), (9000, 0)] {
        let mut phases = phases();
        phases.activated = Some(Duration::ZERO);
        phases.postroll = Duration::from_millis(postroll);
        let mut health = Health {
            graph_started: true,
            recorder_running: true,
            source_running: true,
            ..Default::default()
        };
        assert_eq!(
            phases.tick(Duration::from_secs(1), health).unwrap(),
            Action::Wait
        );
        health.source_submitted = true;
        assert!(
            phases.tick(Duration::from_millis(millis), health).is_err(),
            "first Submitted observed at {millis}ms bypassed 7s + 1s deadline"
        );
    }
}

#[test]
fn process_collection_timely_acks_allow_postroll_after_first_ack_deadlines() {
    let mut phases = phases();
    phases.activated = Some(Duration::ZERO);
    let mut health = Health {
        graph_started: true,
        recorder_running: true,
        source_running: true,
        ..Default::default()
    };
    assert_eq!(
        phases.tick(Duration::from_millis(4999), health).unwrap(),
        Action::Wait
    );
    health.source_submitted = true;
    assert_eq!(
        phases.tick(Duration::from_millis(7999), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_millis(8099), health).unwrap(),
        Action::Complete
    );
}

#[test]
fn process_collection_combined_late_first_acks_cannot_complete_zero_postroll() {
    for millis in [5000, 5001, 8000, 9000] {
        let mut phases = phases();
        phases.activated = Some(Duration::ZERO);
        phases.postroll = Duration::ZERO;
        let result = phases.tick(
            Duration::from_millis(millis),
            Health {
                graph_started: true,
                recorder_running: true,
                source_running: true,
                source_submitted: true,
                ..Default::default()
            },
        );
        assert!(
            result.is_err(),
            "combined first acks at {millis}ms completed late"
        );
        assert_eq!(phases.running, None);
        assert_eq!(phases.submitted, None);
    }
}

#[test]
fn process_collection_long_source_keeps_timely_running_ack_past_start_deadline() {
    let mut phases = phases();
    phases.activated = Some(Duration::ZERO);
    phases.source_duration = Duration::from_secs(10);
    phases.ceiling = Duration::from_secs(30);
    let mut health = Health {
        graph_started: true,
        recorder_running: true,
        source_running: true,
        ..Default::default()
    };
    assert_eq!(
        phases.tick(Duration::from_secs(1), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_secs(10), health).unwrap(),
        Action::Wait
    );
    health.source_submitted = true;
    assert_eq!(
        phases.tick(Duration::from_millis(16_999), health).unwrap(),
        Action::Wait
    );
    assert_eq!(
        phases.tick(Duration::from_millis(17_099), health).unwrap(),
        Action::Complete
    );
}

#[test]
fn process_collection_first_graph_recorder_start_observed_late_cannot_bypass_deadline() {
    let mut phases = phases();
    assert!(
        phases
            .tick(
                Duration::from_secs(5),
                Health {
                    graph_started: true,
                    recorder_running: true,
                    ..Default::default()
                }
            )
            .is_err()
    );
}

#[test]
fn process_collection_retirement_broadcasts_all_stops_despite_error_and_panic() {
    let mut seen = Vec::new();
    let mut failures = None;
    retire(
        |stage| {
            seen.push(stage);
            if stage == Retirement::StopGraph {
                anyhow::bail!("graph stop rejected");
            }
            if stage == Retirement::StopRecorder {
                panic!("recorder stop panic");
            }
            Ok(())
        },
        &mut failures,
    );
    assert_eq!(
        seen,
        [
            Retirement::StopGraph,
            Retirement::StopRecorder,
            Retirement::StopSource,
            Retirement::StopLoad,
            Retirement::JoinGraph,
            Retirement::FinishSource,
            Retirement::FinishLoad,
            Retirement::FinishRecorder
        ]
    );
    let failures = failures.unwrap();
    assert!(failures.contains("graph stop rejected") && failures.contains("panic"));
}

fn observation() -> Value {
    json!({"capture_started":true,"render_started":true,"capture_finished":false,"render_finished":false,
        "timeline":{"epoch":1,"revision":1,"frame":0},
        "bridge":{"dropped_frames":0,"nonfinite_samples":0,"discontinuities":0,"timestamp_errors":0,
            "backlog_discarded_frames":0,"age_discarded_frames":0,"underrun_frames":0,"resets":0}})
}

#[cfg(windows)]
fn active_observation() -> Value {
    let mut value = observation();
    value["bridge"]["native_delivery"] = json!({"packet_bound_violations":0,
        "gap_bound_violations":0,"unavailable_clocks":0,"regressed_clocks":0});
    value
}

#[cfg(windows)]
fn finish_observation(final_value: &Value) -> Guard {
    let mut guard = Guard::default();
    let before = active_observation();
    assert!(validate_and_latch(&before, true, &mut guard.first_failure).is_ok());
    guard.last_observation = Some(before);
    guard.finish_with(|guard, stage| {
        if stage == Retirement::JoinGraph {
            guard.final_observation = Some(final_value.clone());
        }
        Ok(())
    });
    guard
}

#[test]
fn process_collection_active_missing_delivery_evidence_is_not_zero_faults() {
    assert!(
        validate_graph_observation(&observation(), true).is_err(),
        "missing live delivery must fail closed"
    );
    assert!(
        validate_graph_observation(&observation(), false).is_ok(),
        "pre-source Start must not require a packet or advancing frame"
    );
}

#[test]
fn process_collection_setup_cannot_activate_without_remaining_source_budget() {
    let mut phases = phases();
    phases.ceiling = Duration::from_secs(8);
    let health = Health {
        graph_started: true,
        recorder_running: true,
        ..Default::default()
    };
    assert!(phases.tick(Duration::from_secs(7), health).is_err());
}

#[test]
fn process_collection_config_caps_and_unknown_fields_reject_before_collection() {
    let config: Config =
        serde_json::from_value(json!({"schema_version":1,"input_endpoint_id":"input",
        "ring_frames":1024,"max_pcm_bytes":1048576,"max_metadata_bytes":65536,"max_packets":1000,
        "preroll_ms":100,"postroll_ms":100}))
        .unwrap();
    assert!(config.validate(48_000, 14).is_ok());
    assert!(config.validate(48_000, 12).is_err());
    let mut value = serde_json::to_value(&config).unwrap();
    value["source_pid"] = json!(42);
    assert!(serde_json::from_value::<Config>(value).is_err());
    let mut config = config;
    config.ring_frames = usize::MAX;
    assert!(config.validate(48_000, 14).is_err());
}

#[test]
fn process_collection_failure_cap_includes_separator_and_utf8() {
    let mut failure = Some("x".repeat(16383));
    append_failure(&mut failure, "é failure");
    assert_eq!(failure.as_ref().unwrap().len(), 16383);
    let mut failure = Some("x".repeat(16380));
    append_failure(&mut failure, "éé");
    assert_eq!(failure.as_ref().unwrap().len(), 16384);
}

fn final_graph() -> (Value, Value, Value) {
    let identity = json!({"pid":42,"creation_time_100ns":71});
    let capture = json!({"failure":null,"stream_started":true,"stop_succeeded":true,"status":"stopped",
        "sample_rate":48000,"channels":2,"process":identity,"timeout_wakes":0,"packets":10,
        "delivery":{"packet_bound_violations":0,"gap_bound_violations":0,"unavailable_clocks":0,"regressed_clocks":0},
        "artifact_metadata":{"dropped_frames":0,"overflow_events":0,"gap_events":0,"discontinuity_events":0}});
    let render = json!({"failure":null,"stream_started":true,"stop_succeeded":true,"status":"stopped",
        "sample_rate":48000,"channels":2,"stats":{"timeout_wakes":0,"processed_frames":48000,"bridge_shortfall_frames":0}});
    (capture, render, identity)
}

#[test]
fn process_collection_final_graph_rejects_late_failure_stop_timeout_or_changed_identity() {
    let (capture, render, identity) = final_graph();
    assert!(validate_final_graph(&capture, &render, &identity).is_ok());
    for pointer in [
        "/failure",
        "/stop_succeeded",
        "/timeout_wakes",
        "/process/creation_time_100ns",
    ] {
        let mut changed = capture.clone();
        *changed.pointer_mut(pointer).unwrap() = match pointer {
            "/failure" => json!("late native fault"),
            "/stop_succeeded" => json!(false),
            _ => json!(1),
        };
        assert!(
            validate_final_graph(&changed, &render, &identity).is_err(),
            "missed final {pointer}"
        );
    }
    let mut render = render;
    render["stats"]["bridge_shortfall_frames"] = json!(1);
    assert!(validate_final_graph(&capture, &render, &identity).is_err());
}

#[test]
fn process_collection_first_retirement_failure_is_latched_before_distinct_next_observation() {
    let mut first = None;
    let mut raw = observation();
    raw["bridge"]["underrun_frames"] = json!(3);
    assert!(validate_and_latch(&raw, false, &mut first).is_err());
    let mut later = raw.clone();
    later["bridge"]["underrun_frames"] = json!(7);
    assert!(validate_and_latch(&later, false, &mut first).is_err());
    assert_eq!(first.unwrap()["bridge"]["underrun_frames"], 3);
    assert_eq!(later["bridge"]["underrun_frames"], 7);
}

#[cfg(windows)]
#[test]
fn process_collection_actual_guard_reconciles_final_only_bridge_faults_after_stops() {
    for key in [
        "dropped_frames",
        "nonfinite_samples",
        "discontinuities",
        "timestamp_errors",
        "underrun_frames",
        "resets",
        "backlog_discarded_frames",
        "age_discarded_frames",
    ] {
        let mut final_value = active_observation();
        final_value["capture_finished"] = json!(true);
        final_value["render_finished"] = json!(true);
        final_value["bridge"][key] = json!(1);
        let guard = finish_observation(&final_value);
        assert!(
            guard
                .failures
                .as_deref()
                .is_some_and(|error| error.contains(key)),
            "final-only {key} was retained without invalidating collection"
        );
        assert_eq!(guard.first_failure.as_ref().unwrap()["bridge"][key], 1);
        assert_eq!(guard.final_observation.as_ref().unwrap(), &final_value);
    }
}

#[cfg(windows)]
#[test]
fn process_collection_actual_guard_rejects_final_timeline_drift_and_missing_identity() {
    for pointer in ["/timeline/epoch", "/timeline/revision", "/timeline"] {
        for replacement in [json!(2), Value::Null] {
            let mut final_value = active_observation();
            *final_value.pointer_mut(pointer).unwrap() = replacement;
            let guard = finish_observation(&final_value);
            assert!(
                guard.failures.is_some(),
                "final {pointer} was accepted without valid identity"
            );
            assert_eq!(guard.first_failure, Some(final_value.clone()));
            assert_eq!(guard.final_observation, Some(final_value));
        }
    }
    for key in ["epoch", "revision", "timeline"] {
        let mut final_value = active_observation();
        if key == "timeline" {
            final_value.as_object_mut().unwrap().remove(key);
        } else {
            final_value["timeline"].as_object_mut().unwrap().remove(key);
        }
        let guard = finish_observation(&final_value);
        assert!(guard.failures.is_some(), "missing final {key} was accepted");
        assert_eq!(guard.first_failure, Some(final_value.clone()));
        assert_eq!(guard.final_observation, Some(final_value));
    }
}

#[cfg(windows)]
#[test]
fn process_collection_actual_guard_rejects_final_delivery_faults_or_missing_fields() {
    for pointer in [
        "/bridge",
        "/bridge/dropped_frames",
        "/bridge/nonfinite_samples",
        "/bridge/discontinuities",
        "/bridge/timestamp_errors",
        "/bridge/underrun_frames",
        "/bridge/resets",
        "/bridge/backlog_discarded_frames",
        "/bridge/age_discarded_frames",
        "/bridge/native_delivery",
        "/bridge/native_delivery/packet_bound_violations",
        "/bridge/native_delivery/gap_bound_violations",
        "/bridge/native_delivery/unavailable_clocks",
        "/bridge/native_delivery/regressed_clocks",
    ] {
        let mut final_value = active_observation();
        *final_value.pointer_mut(pointer).unwrap() = Value::Null;
        let guard = finish_observation(&final_value);
        assert!(
            guard.failures.is_some(),
            "missing final {pointer} was accepted"
        );
        assert_eq!(guard.first_failure, Some(final_value.clone()));
        assert_eq!(guard.final_observation, Some(final_value));
    }
    for key in [
        "packet_bound_violations",
        "gap_bound_violations",
        "unavailable_clocks",
        "regressed_clocks",
    ] {
        let mut final_value = active_observation();
        final_value["bridge"]["native_delivery"][key] = json!(1);
        let guard = finish_observation(&final_value);
        assert!(
            guard
                .failures
                .as_deref()
                .is_some_and(|error| error.contains(key)),
            "final delivery {key} was accepted"
        );
        assert_eq!(guard.first_failure, Some(final_value.clone()));
        assert_eq!(guard.final_observation, Some(final_value));
    }
}

#[cfg(windows)]
#[test]
fn process_collection_actual_guard_accepts_clean_final_with_stopped_or_absent_owners() {
    for owner_state in [json!(true), Value::Null] {
        let mut final_value = active_observation();
        final_value["capture_finished"] = owner_state.clone();
        final_value["render_finished"] = owner_state;
        final_value["capture_started"] = Value::Null;
        final_value["render_started"] = Value::Null;
        let guard = finish_observation(&final_value);
        assert!(
            guard.failures.is_none(),
            "joined owner liveness is not final evidence"
        );
        assert_eq!(guard.first_failure, None);
        assert_eq!(guard.final_observation, Some(final_value));
    }
}

#[cfg(windows)]
#[test]
fn process_collection_final_bridge_fault_keeps_earlier_failure_and_distinct_final_raw() {
    let mut guard = Guard::default();
    let mut first = observation();
    first["bridge"]["underrun_frames"] = json!(3);
    assert!(validate_and_latch(&first, false, &mut guard.first_failure).is_err());
    let mut final_value = first.clone();
    final_value["capture_finished"] = json!(true);
    final_value["render_finished"] = json!(true);
    final_value["bridge"]["underrun_frames"] = json!(7);
    guard.finish_with(|guard, stage| {
        if stage == Retirement::JoinGraph {
            guard.final_observation = Some(final_value.clone());
        }
        Ok(())
    });
    assert!(
        guard.failures.is_some(),
        "final bridge fault must fail without replacing first raw"
    );
    assert_eq!(guard.first_failure, Some(first));
    assert_eq!(guard.final_observation, Some(final_value));
}

#[cfg(windows)]
#[test]
fn process_collection_actual_guard_preserves_partial_reports_after_run_panic_and_stops_before_joins()
 {
    let mut guard = Guard::default();
    let result = guarded_run(&mut guard, |guard| {
        guard.source_outcome = Some(super::super::process_source::Outcome {
            child_pid: Some(42),
            failure: Some("ready failed".into()),
            ..Default::default()
        });
        panic!("injected partial graph compile panic");
    });
    assert!(result.unwrap_err().to_string().contains("control panic"));
    let mut seen = Vec::new();
    guard.finish_with(|guard, stage| {
        seen.push(stage);
        if stage == Retirement::StopRecorder {
            anyhow::bail!("stop recorder failed");
        }
        if stage == Retirement::JoinGraph {
            guard.final_observation = Some(json!({"partial_capture":"retained"}));
            panic!("injected graph join panic");
        }
        Ok(())
    });
    assert_eq!(seen.len(), 8);
    assert_eq!(
        &seen[..4],
        &[
            Retirement::StopGraph,
            Retirement::StopRecorder,
            Retirement::StopSource,
            Retirement::StopLoad
        ]
    );
    assert_eq!(guard.source_outcome.as_ref().unwrap().child_pid, Some(42));
    assert_eq!(
        guard.final_observation.as_ref().unwrap()["partial_capture"],
        "retained"
    );
    assert!(
        guard
            .failures
            .as_ref()
            .unwrap()
            .contains("stop recorder failed")
    );
    assert!(guard.failures.as_ref().unwrap().contains("cleanup panic"));
    assert!(guard.retired);
}

#[cfg(windows)]
#[test]
fn process_collection_real_library_validation_abort_calls_b_peer_stop_before_return() {
    use moiren_windows_audio::{
        ActivationGate, SessionDuration, StopSignal,
        process_loopback::{
            ProcessIdentity, ProcessLoopbackOptions,
            prepare_process_capture_with_gate_returning_with_abort,
        },
    };
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let mut guard = Guard::default();
    let mut calls = 0;
    let result = prepare_process_capture_with_gate_returning_with_abort(
        ProcessLoopbackOptions {
            artifact: None,
            bridge: Default::default(),
            latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
            target: ProcessIdentity {
                pid: 0,
                creation_time_100ns: 0,
                executable_name: "invalid".into(),
            },
            duration: SessionDuration::UntilStopped,
        },
        stop,
        gate,
        || {
            calls += 1;
            guard.stop_peers();
            guard.failures.clone()
        },
    );
    let failure = match result {
        Err(error) => error,
        Ok(_) => panic!("invalid PID must fail before native startup"),
    };
    assert_eq!(calls, 1);
    assert!(!failure.abort_panicked);
    assert!(failure.report.is_none());
    guard.finish();
}

#[cfg(windows)]
#[test]
fn process_collection_shared_renderer_preserves_epoch_revision_without_rendering() {
    let (_ingress, source, _observer) =
        moiren_windows_audio::clock_bridge::capture_bridge(Default::default()).unwrap();
    let renderer = graph::build_renderer(&super::super::LiveOptions::default(), source).unwrap();
    let timeline = renderer.timeline_observer().try_snapshot().unwrap();
    assert_eq!(
        (timeline.epoch, timeline.revision, timeline.frame),
        (1, 1, 0)
    );
    drop(renderer);
}

#[cfg(windows)]
#[test]
fn process_collection_pcm_open_write_or_flush_failure_still_attempts_metadata() {
    use moiren_windows_audio::capture::{PcmArtifactConfig, PcmRecorder};
    use std::io::{self, Write};
    struct FailedWriter {
        write: bool,
    }
    impl Write for FailedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.write {
                Err(io::Error::other("injected PCM write failure"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("injected PCM flush failure"))
        }
    }
    let mut recorder = PcmRecorder::new(PcmArtifactConfig::new(1), 48_000, 2).unwrap();
    recorder.record_packet(&[0; 8], 1, 0, 0).unwrap();
    let artifact = recorder.finish();
    for failure in ["open", "write", "flush"] {
        let directory = std::env::temp_dir().join(format!(
            "moiren-b-artifact-{}-{failure}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let paths = super::super::ArtifactPaths {
            capture_pcm: Some(
                directory
                    .join("capture.f32le")
                    .to_string_lossy()
                    .into_owned(),
            ),
            capture_pcm_metadata: Some(
                directory
                    .join("capture.metadata.json")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        };
        let result = graph::persist_artifact_with(&paths, &artifact, |_| {
            if failure == "open" {
                anyhow::bail!("injected PCM open failure");
            }
            Ok(FailedWriter {
                write: failure == "write",
            })
        });
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains(&format!("injected PCM {failure} failure")));
        let metadata = directory.join("capture.metadata.json");
        let attempted = metadata.is_file();
        let raw = std::fs::read(&metadata).ok();
        let _ = std::fs::remove_file(&metadata);
        std::fs::remove_dir(&directory).unwrap();
        assert!(attempted, "PCM {failure} failure skipped metadata attempt");
        let value: Value = serde_json::from_slice(&raw.unwrap()).unwrap();
        assert_eq!(value["stored_frames"], 1);
        assert_eq!(
            artifact.metadata().stored_frames,
            1,
            "artifact owner must remain available"
        );
    }
}

#[cfg(windows)]
#[test]
fn process_collection_metadata_failure_retains_pcm_and_dual_errors_invalidate_health() {
    use moiren_windows_audio::capture::{PcmArtifactConfig, PcmRecorder};
    use std::fs::OpenOptions;
    let mut recorder = PcmRecorder::new(PcmArtifactConfig::new(1), 48_000, 2).unwrap();
    recorder.record_packet(&[0; 8], 1, 0, 0).unwrap();
    let artifact = recorder.finish();
    for fail_pcm in [false, true] {
        let directory = std::env::temp_dir().join(format!(
            "moiren-b-artifact-metadata-{}-{fail_pcm}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let pcm_path = directory.join("capture.f32le");
        let metadata_path = directory.join("capture.metadata.json");
        std::fs::write(&metadata_path, b"retained original metadata").unwrap();
        let paths = super::super::ArtifactPaths {
            capture_pcm: Some(pcm_path.to_string_lossy().into_owned()),
            capture_pcm_metadata: Some(metadata_path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let mut pcm_attempted = false;
        let result = graph::persist_artifact_with(&paths, &artifact, |path| {
            pcm_attempted = true;
            if fail_pcm {
                anyhow::bail!("injected PCM open failure alongside metadata failure");
            }
            Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
        });
        let pcm = std::fs::read(&pcm_path).ok();
        let metadata = std::fs::read(&metadata_path).unwrap();
        if pcm.is_some() {
            std::fs::remove_file(&pcm_path).unwrap();
        }
        std::fs::remove_file(&metadata_path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        assert!(pcm_attempted, "metadata failure skipped PCM attempt");
        assert_eq!(pcm.as_deref(), (!fail_pcm).then_some(artifact.pcm_bytes()));
        assert_eq!(
            metadata, b"retained original metadata",
            "must not overwrite failed raw evidence"
        );
        assert_eq!(
            artifact.metadata().stored_frames,
            1,
            "typed artifact stays owned"
        );
        let mut guard = Guard::default();
        assert!(guard.collection_complete(true));
        guard.record_graph_persistence(result);
        assert!(
            !guard.collection_complete(true),
            "artifact failure must invalidate collection health"
        );
        let error = guard.failures.as_deref().unwrap();
        assert!(error.contains("write capture PCM metadata"));
        assert_eq!(
            error.contains("injected PCM open failure"),
            fail_pcm,
            "both independent failures must survive aggregation: {error}"
        );
    }
}
