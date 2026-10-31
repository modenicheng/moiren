use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-owned-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        // Only this fixture's freshly created directory is ever removed.
        regular_path(&self.0).unwrap();
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn owned_process_source_ipc_is_bounded_immutable_and_collision_safe() {
    let root = Directory::new();
    let directory = create_run_directory(&root.0.join("child")).unwrap();
    assert!(create_run_directory(&directory).is_err());
    let value = receipt(Phase::Ready);
    publish(&directory, Phase::Ready.file(), &value).unwrap();
    let path = directory.join(Phase::Ready.file());
    let first = fs::read(&path).unwrap();
    let mut changed = value.clone();
    changed.identity.pid += 1;
    assert!(publish(&directory, Phase::Ready.file(), &changed).is_err());
    assert_eq!(fs::read(&path).unwrap(), first);
    assert_eq!(read_receipt(&path).unwrap().identity.pid, 17);
    assert!(read_receipt(&directory.join("missing")).is_err());
    assert!(read_bounded::<Receipt>(&path, 0).is_err());
    assert!(read_bounded::<Receipt>(&directory, RECEIPT_CAP).is_err());
    fs::write(directory.join("bad"), b"{bad}").unwrap();
    assert!(read_receipt(&directory.join("bad")).is_err());
    fs::write(directory.join("huge"), vec![b' '; RECEIPT_CAP as usize + 1]).unwrap();
    assert!(read_receipt(&directory.join("huge")).is_err());
    assert!(
        publish(
            &directory,
            "too-large.json",
            &"x".repeat(RECEIPT_CAP as usize)
        )
        .is_err()
    );
    assert!(!directory.join("too-large.json").exists());
    assert!(!has_token(&directory, "activate").unwrap());
    token(&directory, "activate").unwrap();
    assert!(has_token(&directory, "activate").unwrap());
    assert!(token(&directory, "activate").is_err());
    fs::write(directory.join("stop"), b"11").unwrap();
    assert!(has_token(&directory, "stop").is_err());
}

#[test]
fn owned_process_source_receipt_budget_covers_max_markers_or_fails_before_launch() {
    let mut schedule = receipt(Phase::Ready).schedule;
    schedule.starts = (0..4096).map(|i| i * 64).collect();
    schedule.total_source_frames = 4096 * 64;
    let mut config = config();
    config.seconds = 600;
    config.source_endpoint = "\0".repeat(4096);
    assert!(validate_receipt_budget(&config, &schedule).is_ok());
    schedule.schedule_id = "x".repeat(RECEIPT_CAP as usize);
    assert!(validate_receipt_budget(&config, &schedule).is_err());
}

#[test]
fn owned_process_source_observation_keeps_invalid_receipt_without_acceptance() {
    let ready = receipt(Phase::Ready);
    let mut progress = Progress::default();
    progress
        .observe(
            ready.clone(),
            Phase::Ready,
            false,
            &ready.identity,
            &config(),
            &ready.schedule,
        )
        .unwrap();
    let running = receipt(Phase::Running);
    assert!(
        progress
            .observe(
                running,
                Phase::Running,
                false,
                &ready.identity,
                &config(),
                &ready.schedule
            )
            .is_err()
    );
    assert!(progress.running_receipt.is_some() && !progress.running);
    let mut submitted = receipt(Phase::Submitted);
    submitted.identity.creation_time_100ns = 999;
    assert!(
        progress
            .observe(
                submitted,
                Phase::Submitted,
                true,
                &ready.identity,
                &config(),
                &ready.schedule
            )
            .is_err()
    );
    assert_eq!(
        progress
            .submitted_receipt
            .unwrap()
            .identity
            .creation_time_100ns,
        999
    );
    assert!(!progress.submitted);
}

#[test]
fn owned_process_source_deadline_includes_never_activated_time() {
    assert!(check_lifetime(Duration::from_millis(4999), 5).is_ok());
    assert!(check_lifetime(Duration::from_secs(5), 5).is_err());
    assert!(check_lifetime(Duration::from_secs(9), 5).is_err());
}

struct TestChild {
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    exited: bool,
    unconfirmed: bool,
}
impl supervision::ChildOwner for TestChild {
    fn id(&self) -> u32 {
        17
    }
    fn inspect(&self) -> Result<moiren_windows_audio::process_loopback::ProcessIdentity> {
        Ok(moiren_windows_audio::process_loopback::ProcessIdentity {
            pid: 17,
            creation_time_100ns: 1234,
            executable_name: "probe.exe".into(),
        })
    }
    fn try_exit(&mut self) -> Result<Option<Option<i32>>> {
        self.events.borrow_mut().push("try_exit");
        Ok(self.exited.then_some(Some(0)))
    }
    fn kill(&mut self) -> Result<()> {
        self.events.borrow_mut().push("kill");
        if self.unconfirmed {
            anyhow::bail!("kill refused");
        }
        self.exited = true;
        Ok(())
    }
}
struct TestTime(Duration);
impl supervision::Clock for TestTime {
    fn elapsed(&self) -> Duration {
        self.0
    }
    fn poll(&mut self) {
        self.0 += Duration::from_secs(1);
    }
}
fn session(directory: &Directory, unconfirmed: bool) -> session::Session<TestChild> {
    session::Session {
        child: TestChild {
            events: Default::default(),
            exited: false,
            unconfirmed,
        },
        identity: moiren_windows_audio::process_loopback::ProcessIdentity {
            pid: 17,
            creation_time_100ns: 1234,
            executable_name: "probe.exe".into(),
        },
        config: config(),
        schedule: receipt(Phase::Ready).schedule,
        directory: directory.0.clone(),
        progress: Progress::default(),
        activated: false,
        stop_requested: false,
        finished: false,
        elapsed: Duration::ZERO,
        failure: None,
    }
}

#[test]
fn owned_process_source_parent_failed_ready_runs_abort_before_retirement_and_keeps_partial() {
    for malformed in [false, true] {
        let directory = Directory::new();
        if malformed {
            fs::write(directory.0.join("ready.json"), b"{").unwrap();
        }
        let mut session = session(&directory, true);
        let events = session.child.events.clone();
        let mut clock = TestTime(Duration::ZERO);
        assert!(session.await_ready(&mut clock).is_err());
        let before = events.borrow().len();
        let outcome =
            supervision::abort_preparation(&mut || events.borrow_mut().push("peer_stop"), || {
                session.finish(&mut TestTime(Duration::ZERO))
            });
        assert_eq!(events.borrow()[before], "peer_stop");
        assert!(events.borrow()[before + 1..].contains(&"kill"));
        assert_eq!(outcome.child_pid, Some(17));
        assert_eq!(outcome.pinned_identity.unwrap().creation_time_100ns, 1234);
        assert!(!outcome.valid && !outcome.joined && outcome.forced_termination);
        assert_eq!(outcome.child_exit_code, None);
        assert!(outcome.failure.as_deref().unwrap().contains("unconfirmed"));
        assert!(directory.0.join("stop").exists());
    }
}

#[test]
fn owned_process_source_parent_lifecycle_and_late_final_failure_are_real_paths() {
    let directory = Directory::new();
    publish(&directory.0, "ready.json", &receipt(Phase::Ready)).unwrap();
    let mut session = session(&directory, false);
    session.await_ready(&mut TestTime(Duration::ZERO)).unwrap();
    assert!(session.check().unwrap().prepared);
    session.activate().unwrap();
    assert!(session.activate().is_err());
    assert!(has_token(&directory.0, "activate").unwrap());
    for phase in [Phase::Running, Phase::Submitted] {
        publish(&directory.0, phase.file(), &receipt(phase)).unwrap();
    }
    assert!(session.check().unwrap().submitted);
    session.request_stop().unwrap();
    session.request_stop().unwrap();
    let mut final_receipt = receipt(Phase::Final);
    final_receipt.playback["event_timeouts"] = 3.into();
    publish(&directory.0, "final.json", &final_receipt).unwrap();
    session.child.exited = true;
    let outcome = session.finish(&mut TestTime(Duration::ZERO));
    assert!(outcome.joined && !outcome.valid);
    assert!(!outcome.forced_termination);
    assert_eq!(outcome.final_receipt.unwrap().playback["event_timeouts"], 3);
    assert!(outcome.progress.submitted);
}

#[test]
fn owned_process_source_parent_stop_write_error_is_latched_even_after_retry() {
    let directory = Directory::new();
    let mut session = session(&directory, false);
    fs::create_dir(directory.0.join("stop")).unwrap();
    assert!(session.request_stop().is_err());
    fs::remove_dir(directory.0.join("stop")).unwrap();
    session.request_stop().unwrap();
    session.child.exited = true;
    let outcome = session.finish(&mut TestTime(Duration::ZERO));
    assert!(
        outcome.failure.as_deref().unwrap().contains("stop token"),
        "first stop failure was discarded"
    );
}

#[test]
fn owned_process_source_parent_early_exit_and_stopped_activation_fail_closed() {
    let directory = Directory::new();
    publish(&directory.0, "ready.json", &receipt(Phase::Ready)).unwrap();
    let mut session = session(&directory, false);
    session.child.exited = true;
    assert!(session.check().is_err());
    assert!(session.activate().is_err());
    assert!(!directory.0.join("activate").exists());
    let outcome = session.finish(&mut TestTime(Duration::ZERO));
    assert!(outcome.joined && !outcome.valid);
    assert_eq!(outcome.child_exit_code, Some(0));
    assert!(outcome.failure.unwrap().contains("exited before"));

    let other = Directory::new();
    let mut stopped = self::session(&other, false);
    stopped.request_stop().unwrap();
    assert!(stopped.activate().is_err());
    assert!(!other.0.join("activate").exists());
}

#[test]
fn owned_process_source_success_requires_full_observed_phases_and_clean_final() {
    let ready = receipt(Phase::Ready);
    let mut progress = Progress::default();
    for phase in [Phase::Ready, Phase::Running, Phase::Submitted] {
        progress
            .observe(
                receipt(phase),
                phase,
                true,
                &ready.identity,
                &config(),
                &ready.schedule,
            )
            .unwrap();
    }
    let mut outcome = Outcome {
        child_pid: Some(17),
        pinned_identity: Some(ready.identity),
        progress,
        final_receipt: Some(receipt(Phase::Final)),
        child_exit_code: Some(0),
        joined: true,
        ..Outcome::default()
    };
    finalize(&mut outcome);
    assert!(outcome.valid);
    outcome.forced_termination = true;
    finalize(&mut outcome);
    assert!(!outcome.valid);
}

#[test]
fn owned_process_source_rejects_malformed_cli_and_duration() {
    let args = |seconds: &str| {
        vec![
            "--source-endpoint".into(),
            "Exact Endpoint ".into(),
            "--source-schedule".into(),
            "schedule.json".into(),
            "--run-directory".into(),
            "new dir".into(),
            "--seconds".into(),
            seconds.into(),
        ]
    };
    assert!(parse(args("5")).is_ok());
    for raw in ["0", "601", "+5", " 5", "5 ", "1.0", "18446744073709551616"] {
        assert!(parse(args(raw)).is_err(), "accepted {raw}");
    }
    let mut duplicate = args("5");
    duplicate[2] = "--source-endpoint".into();
    assert!(parse(duplicate).is_err());
}

#[test]
fn owned_process_source_worker_protocol_accepts_explicit_correctness_policy() {
    let arguments = [
        "--source-endpoint",
        "Exact Endpoint ",
        "--source-schedule",
        "schedule.json",
        "--run-directory",
        "new dir",
        "--seconds",
        "5",
        "--validation-mode",
        "correctness",
        "--jitter-tolerance-ms",
        "3000",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert!(
        parse(arguments).is_ok(),
        "the owned child must receive the parent's validation intent"
    );
}

#[test]
fn source_write_witness_switch_preserves_values_and_is_worker_only() {
    let args = || {
        [
            "--source-endpoint",
            "endpoint",
            "--source-schedule",
            "schedule.json",
            "--run-directory",
            "new-dir",
            "--seconds",
            "5",
        ]
        .map(str::to_owned)
        .to_vec()
    };
    assert!(!parse(args()).unwrap().2);
    for index in [1, 3, 5] {
        let mut values = args();
        values[index] = "--source-write-witness".into();
        let (config, directory, enabled) = parse(values).unwrap();
        assert!(!enabled, "a value must not become a switch");
        let retained = match index {
            1 => config.source_endpoint,
            3 => config.source_schedule.to_str().unwrap().to_owned(),
            _ => directory.to_str().unwrap().to_owned(),
        };
        assert_eq!(retained, "--source-write-witness");
    }
    for index in [0, 4, 8] {
        let mut values = args();
        values.insert(index, "--source-write-witness".into());
        assert!(parse(values.clone()).unwrap().2);
        values.push("--source-write-witness".into());
        assert!(parse(values).is_err());
    }
    assert!(
        super::super::super::cli::run(vec!["analyze-pcm".into(), "--source-write-witness".into(),])
            .is_err()
    );
}

#[test]
fn owned_process_source_correctness_receipts_admit_timeouts_but_not_policy_or_identity_changes() {
    let policy = ValidationPolicy {
        mode: ValidationMode::Correctness,
        jitter_tolerance_ms: 3000,
    };
    let mut config = config();
    config.validation = policy;
    assert!(validate_event_timeouts(1, ValidationPolicy::default()).is_err());
    assert!(validate_event_timeouts(9, policy).is_ok());
    for phase in [Phase::Running, Phase::Submitted, Phase::Final] {
        let mut value = receipt(phase);
        value.validation = policy;
        value.playback["event_timeouts"] = 9.into();
        assert!(validate_receipt(&value, phase, &value.identity, &config, &value.schedule).is_ok());
        let expected_identity = value.identity.clone();
        value.identity.creation_time_100ns += 1;
        assert!(
            validate_receipt(&value, phase, &expected_identity, &config, &value.schedule).is_err()
        );
        value.identity = expected_identity;
        value.validation = ValidationPolicy::default();
        assert!(
            validate_receipt(&value, phase, &value.identity, &config, &value.schedule).is_err()
        );
    }
}

#[test]
fn owned_process_source_correctness_parent_continues_and_retains_raw_timeout_quality() {
    let directory = Directory::new();
    let policy = ValidationPolicy {
        mode: ValidationMode::Correctness,
        jitter_tolerance_ms: 3000,
    };
    let mut source = session(&directory, false);
    source.config.validation = policy;
    let mut ready = receipt(Phase::Ready);
    ready.validation = policy;
    publish(&directory.0, "ready.json", &ready).unwrap();
    source.await_ready(&mut TestTime(Duration::ZERO)).unwrap();
    source.activate().unwrap();
    for (phase, count) in [(Phase::Running, 2), (Phase::Submitted, 5)] {
        let mut value = receipt(phase);
        value.validation = policy;
        value.playback["event_timeouts"] = count.into();
        publish(&directory.0, phase.file(), &value).unwrap();
        source.check().unwrap();
    }
    assert!(source.progress.submitted);
    assert!(
        !directory.0.join("stop").exists(),
        "soft timeout must not stop the producer"
    );
    source.request_stop().unwrap();
    let mut final_value = receipt(Phase::Final);
    final_value.validation = policy;
    final_value.playback["event_timeouts"] = 11.into();
    publish(&directory.0, "final.json", &final_value).unwrap();
    source.child.exited = true;
    let outcome = source.finish(&mut TestTime(Duration::ZERO));
    assert!(outcome.valid && outcome.joined && outcome.failure.is_none());
    assert!(!outcome.quality_passed);
    assert_eq!(outcome.validation, policy);
    assert_eq!(
        outcome.progress.running_receipt.unwrap().playback["event_timeouts"],
        2
    );
    assert_eq!(
        outcome.progress.submitted_receipt.unwrap().playback["event_timeouts"],
        5
    );
    assert_eq!(
        outcome.final_receipt.unwrap().playback["event_timeouts"],
        11
    );
}

#[test]
fn owned_process_source_correctness_retains_hard_frame_native_stop_and_storage_checks() {
    let policy = ValidationPolicy {
        mode: ValidationMode::Correctness,
        jitter_tolerance_ms: 3000,
    };
    let mut config = config();
    config.validation = policy;
    for fault in [
        "frames",
        "stop",
        "native",
        "missing_timeouts",
        "counter_type",
    ] {
        let mut value = receipt(Phase::Final);
        value.validation = policy;
        value.playback["event_timeouts"] = 7.into();
        match fault {
            "frames" => {
                value.playback["source_progress"]["acknowledged_source_frames"] = 9999.into()
            }
            "stop" => value.playback["native_stop_succeeded"] = false.into(),
            "native" => value.playback["failure"] = "ReleaseBuffer failed".into(),
            "missing_timeouts" => {
                value
                    .playback
                    .as_object_mut()
                    .unwrap()
                    .remove("event_timeouts");
            }
            "counter_type" => value.playback["event_timeouts"] = "7".into(),
            _ => unreachable!(),
        }
        assert!(
            validate_receipt(
                &value,
                Phase::Final,
                &value.identity,
                &config,
                &value.schedule
            )
            .is_err(),
            "{fault} softened"
        );
    }
    let mut arguments = vec![
        "--source-endpoint",
        "exact",
        "--source-schedule",
        "plan",
        "--run-directory",
        "dir",
        "--seconds",
        "5",
        "--validation-mode",
        "correctness",
        "--jitter-tolerance-ms",
        "3001",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert!(parse(arguments.clone()).is_err());
    *arguments.last_mut().unwrap() = "3000".into();
    arguments.extend(["--validation-mode".into(), "latency-sensitive".into()]);
    assert!(parse(arguments).is_err());
}

#[test]
fn owned_process_source_legacy_protocol_defaults_strict_and_correctness_never_renews_lifetime() {
    let args = [
        "--source-endpoint",
        "exact",
        "--source-schedule",
        "plan",
        "--run-directory",
        "dir",
        "--seconds",
        "5",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        parse(args).unwrap().0.validation,
        ValidationPolicy::default()
    );
    let mut serialized = serde_json::to_value(receipt(Phase::Running)).unwrap();
    serialized.as_object_mut().unwrap().remove("validation");
    assert_eq!(
        serde_json::from_value::<Receipt>(serialized)
            .unwrap()
            .validation,
        ValidationPolicy::default()
    );
    let directory = Directory::new();
    let mut source = session(&directory, false);
    source.config.validation = ValidationPolicy {
        mode: ValidationMode::Correctness,
        jitter_tolerance_ms: 3000,
    };
    source.elapsed = Duration::from_secs(5);
    assert!(
        source
            .check()
            .unwrap_err()
            .to_string()
            .contains("finite lifetime")
    );
}

fn receipt(phase: Phase) -> Receipt {
    Receipt {
        schema_version: 1,
        phase,
        identity: Identity {
            pid: 17,
            creation_time_100ns: 1234,
            executable_name: "probe.exe".into(),
        },
        source_endpoint: "Exact Endpoint ".into(),
        seconds: 5,
        validation: ValidationPolicy::default(),
        schedule: super::super::source::SourceSchedule {
            schema_version: 1,
            schedule_id: "owned-test".into(),
            waveform_version: super::super::source::WAVEFORM_VERSION.into(),
            sample_rate: 48_000,
            channels: 2,
            marker_frames: 32,
            marker_amplitude: 0.1,
            total_source_frames: 64,
            starts: vec![0],
        },
        playback: match phase {
            Phase::Ready => {
                serde_json::json!({"mmdevice_id":"Exact Endpoint ","source_mode":"shared_explicit_mmdevice","native_buffer_frames":480,"packet_format":"Float32"})
            }
            Phase::Running | Phase::Submitted => {
                serde_json::json!({"started":true,"event_timeouts":0,"stopped":false,"source":{"native_preroll_frames":480,"native_submitted_frames":544,"origin_submission_frame":480,"acknowledged_source_frames":64,"schedule_submitted_complete":true}})
            }
            Phase::Final => {
                serde_json::json!({"started":true,"stopped":true,"worker_joined":true,"event_timeouts":0,"native_stop_attempted":true,"native_stop_succeeded":true,"failure":null,"source_progress":{"native_preroll_frames":480,"native_submitted_frames":544,"origin_submission_frame":480,"acknowledged_source_frames":64,"schedule_submitted_complete":true}})
            }
        },
        stop_requested: phase == Phase::Final,
        failure: None,
    }
}

fn config() -> Config {
    Config {
        source_endpoint: "Exact Endpoint ".into(),
        source_schedule: "schedule.json".into(),
        seconds: 5,
        validation: ValidationPolicy::default(),
    }
}

#[test]
fn owned_process_source_receipts_reject_recycled_identity_and_late_faults() {
    let baseline = receipt(Phase::Final);
    assert!(
        validate_receipt(
            &baseline,
            Phase::Final,
            &baseline.identity,
            &config(),
            &baseline.schedule
        )
        .is_ok()
    );
    for field in [
        "identity", "endpoint", "phase", "timeout", "stop", "partial", "failure",
    ] {
        let mut value = baseline.clone();
        match field {
            "identity" => value.identity.creation_time_100ns += 1,
            "endpoint" => value.source_endpoint = "Exact Endpoint".into(),
            "phase" => value.phase = Phase::Submitted,
            "timeout" => value.playback["event_timeouts"] = 1.into(),
            "stop" => value.playback["native_stop_succeeded"] = false.into(),
            "partial" => {
                value.playback["source_progress"]["schedule_submitted_complete"] = false.into()
            }
            "failure" => value.failure = Some("late ReleaseBuffer".into()),
            _ => unreachable!(),
        }
        assert!(
            validate_receipt(
                &value,
                Phase::Final,
                &baseline.identity,
                &config(),
                &baseline.schedule
            )
            .is_err(),
            "accepted {field}"
        );
    }
}

#[test]
fn owned_process_source_partial_evidence_and_forced_exit_never_become_valid() {
    let mut outcome = Outcome {
        validation: ValidationPolicy::default(),
        quality_passed: false,
        source_endpoint: None,
        seconds: None,
        schedule: None,
        child_pid: None,
        pinned_identity: None,
        progress: Progress::default(),
        final_receipt: None,
        child_exit_code: Some(0),
        forced_termination: false,
        joined: true,
        valid: false,
        failure: None,
    };
    finalize(&mut outcome);
    assert!(
        !outcome.valid,
        "zero child exit is not completed source evidence"
    );
}

#[test]
fn owned_process_source_missing_native_failure_field_is_not_success() {
    let mut value = receipt(Phase::Final);
    value.playback.as_object_mut().unwrap().remove("failure");
    assert!(
        validate_receipt(
            &value,
            Phase::Final,
            &value.identity,
            &config(),
            &value.schedule
        )
        .is_err()
    );
}

#[test]
fn owned_process_source_ready_cannot_claim_stop() {
    let mut value = receipt(Phase::Ready);
    value.stop_requested = true;
    assert!(
        validate_receipt(
            &value,
            Phase::Ready,
            &value.identity,
            &config(),
            &value.schedule
        )
        .is_err()
    );
}
