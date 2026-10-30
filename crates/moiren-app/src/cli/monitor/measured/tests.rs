use super::*;

#[test]
fn measured_collection_has_a_finite_unrenewed_observation_deadline() {
    let mut measurement = Measurement {
        seconds: Some(1),
        target: Some((2048, 256, 0)),
        window_policy: None,
        stage: Stage::Collecting {
            since: Instant::now() - Duration::from_secs(2),
            generation: 1,
            revision: 1,
            native: 1,
        },
    };
    assert!(measurement.startup_expired(Instant::now(), 30));
    if let Stage::Collecting { since, .. } = &mut measurement.stage {
        *since = Instant::now();
    }
    assert!(!measurement.startup_expired(Instant::now(), 30));
}

#[cfg(windows)]
fn fixture() -> (AppSnapshot, MonitorReport, MeasuredProcessRequest) {
    let mut options = MonitorOptions::default();
    options.pid = Some(42);
    options.output = Some("output".into());
    let source = InputSelection::Process {
        pid: 42,
        creation_time_100ns: 123,
    };
    let (context, _) = context(&options, source).unwrap();
    let mut snapshot = crate::service::snapshot(&context);
    snapshot.phase = SessionPhase::Running;
    snapshot.generation = crate::service::SessionGeneration(2);
    let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
    graph.active_revision = Some(graph.candidate_revision);
    let node = graph.node_of_kind(NodeKind::Source).unwrap().0.id();
    let request = MeasuredProcessRequest {
        node,
        session_generation: crate::service::SessionGeneration(1),
        native_owner_generation: 7,
        graph_revision: graph.candidate_revision,
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    };
    let observed = crate::service::measured_process::tests::observation(node);
    let summary = crate::service::measured_process::MeasuredProcessProfile::from_observation(
        1, request, &observed,
    )
    .unwrap()
    .summary()
    .clone();
    snapshot.active_audio_settings = Some(options.audio.clone());
    snapshot.active_transport_latency = Some(Default::default());
    snapshot.measured_process = Some(summary.clone());
    snapshot.active_measured_process = Some(summary.clone());
    snapshot.native_observations = Some(crate::service::NativeSessionObservations {
        generation: snapshot.generation,
        observation: moiren_windows_audio::engine_backend::NativeObservationSnapshot {
            virtual_clock: None,
            owner_generation: 1,
            running: true,
            streams: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
            render_padding: Vec::new(),
            process_delivery: Vec::new(),
        },
    });
    let report = MonitorReport {
        schema_version: 1,
        qualified: false,
        route: "test",
        process_pid: 42,
        process_creation_time_100ns: 123,
        output_endpoint_id: "output".into(),
        requested_audio: options.audio.clone(),
        requested_transport: Default::default(),
        confirmed_active_audio: Some(options.audio),
        confirmed_active_transport: Some(Default::default()),
        confirmed_generation: Some(2),
        confirmed_revision: Some(request.graph_revision),
        confirmed_native_generation: Some(1),
        requested_measurement_seconds: Some(1),
        requested_measured_target: Some(MeasuredTarget {
            capacity_frames: 2048,
            target_frames: 256,
            jitter_frames: 0,
        }),
        requested_window_policy: None,
        measurement_generation: Some(1),
        measurement_native_generation: Some(7),
        observed_measurement_ms: 1000,
        confirmed_active_measured: Some(summary),
        requested_seconds: 1,
        observed_running_ms: 0,
        run_complete: false,
        cleanup_complete: false,
        last_running_native: None,
        failure: None,
    };
    (snapshot, report, request)
}

#[cfg(windows)]
#[test]
fn late_baseline_or_replacement_confirmation_cannot_bypass_its_deadline() {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    for applying in [false, true] {
        let (snapshot, mut report, request) = fixture();
        let began = Instant::now() - Duration::from_secs(1);
        let mut measurement = Measurement {
            seconds: Some(1),
            target: Some((2048, 256, 0)),
            window_policy: None,
            stage: if applying {
                Stage::Applying {
                    since: began,
                    request,
                    id: RequestId(0),
                }
            } else {
                Stage::Baseline
            },
        };
        let result = measurement.observe(&snapshot, &handle, &mut report, began, 30);
        if result.is_ok() {
            handle.request_exit();
            runtime.join().unwrap();
            panic!("late confirmation accepted: applying={applying}");
        }
        assert!(result.unwrap_err().to_string().contains("startup deadline"));
        assert!(!measurement.ready());
    }
    handle.request_exit();
    runtime.join().unwrap();
}

#[cfg(windows)]
#[test]
fn confirmed_run_requires_the_same_measured_profile_and_native_owner() {
    let (snapshot, report, _) = fixture();
    assert!(confirmed_run_matches(&snapshot, &report));
    for fault in 0..5 {
        let mut changed = snapshot.clone();
        match fault {
            0 => changed.active_measured_process = None,
            1 => {
                changed
                    .active_measured_process
                    .as_mut()
                    .unwrap()
                    .selection_id += 1
            }
            2 => changed.native_observations = None,
            3 => {
                changed
                    .native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .owner_generation += 1
            }
            4 => changed.native_observations.as_mut().unwrap().generation.0 += 1,
            _ => unreachable!(),
        }
        assert!(!confirmed_run_matches(&changed, &report), "fault {fault}");
    }
}

#[test]
fn requested_run_deadline_requests_exit_even_without_a_new_snapshot() {
    let (_, mut report, _) = fixture();
    let since = Instant::now() - Duration::from_secs(1);
    assert!(check_confirmed_run(
        &mut report,
        None,
        since,
        Duration::from_millis(20)
    ));
    assert!(!report.run_complete);
    assert!(report.observed_running_ms >= 1000);
    assert!(report.failure.unwrap().contains("observation unavailable"));
}

fn window_policy() -> crate::service::ProcessWindowPolicy {
    crate::service::ProcessWindowPolicy {
        max_initial_unserved_frames: 0,
        max_initial_prime_discarded_frames: 0,
        min_output_frames: 1024,
        min_output_reads: 4,
        min_output_span_100ns: 100_000,
    }
}

fn window_options() -> MonitorOptions {
    let mut options = MonitorOptions::default();
    for (flag, value) in [
        ("--measure-seconds", "1"),
        ("--target-frames", "256"),
        ("--capacity-frames", "2048"),
        ("--jitter-frames", "0"),
        ("--window-max-initial-unserved-frames", "0"),
        ("--window-max-initial-prime-discarded-frames", "0"),
        ("--window-min-output-frames", "1024"),
        ("--window-min-output-reads", "4"),
        ("--window-min-output-span-100ns", "100000"),
    ] {
        options.set(flag, value).unwrap();
    }
    options
}

#[test]
fn measured_cli_waits_for_optional_samples_without_renewing_collection() {
    for missing in 0..3 {
        let (mut snapshot, mut report, _) = fixture();
        let node = snapshot
            .graph
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        let complete = crate::service::measured_process::tests::observation(node);
        let mut incomplete = complete.clone();
        match missing {
            0 => incomplete.delivery = None,
            1 => incomplete.producer = None,
            2 => incomplete.consumer = None,
            _ => unreachable!(),
        }
        let native = snapshot.native_observations.as_mut().unwrap();
        native.observation.owner_generation = complete.owner_generation;
        native.observation.process_delivery = vec![incomplete];
        let (handle, mailbox) =
            crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
        let mut options = MonitorOptions::default();
        for (flag, value) in [
            ("--measure-seconds", "1"),
            ("--target-frames", "256"),
            ("--capacity-frames", "2048"),
            ("--jitter-frames", "0"),
        ] {
            options.set(flag, value).unwrap();
        }
        let mut measurement = Measurement::new(&options);
        let since = Instant::now() - Duration::from_millis(1001);
        measurement.stage = Stage::Collecting {
            since,
            generation: snapshot.generation.0,
            revision: snapshot.graph.candidate_revision,
            native: complete.owner_generation,
        };
        for _ in 0..2 {
            measurement
                .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
                .expect("an unknown optional snapshot waits within the fixed deadline");
            assert!(
                matches!(measurement.stage, Stage::Collecting { since: anchor, .. } if anchor == since)
            );
            assert!(mailbox.try_request().is_none());
        }
        snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .process_delivery = vec![complete];
        measurement
            .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
            .unwrap();
        assert!(matches!(
            mailbox.try_request(),
            Some(crate::service::AppRequest::ApplyMeasuredProcessBridge(_))
        ));
    }
}

#[test]
fn measured_cli_missing_samples_expire_at_the_original_deadline() {
    let (mut snapshot, mut report, _) = fixture();
    let node = snapshot
        .graph
        .node_of_kind(NodeKind::Source)
        .unwrap()
        .0
        .id();
    let mut observation = crate::service::measured_process::tests::observation(node);
    observation.delivery = None;
    let native = snapshot.native_observations.as_mut().unwrap();
    native.observation.owner_generation = observation.owner_generation;
    native.observation.process_delivery = vec![observation];
    let (handle, mailbox) =
        crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
    let mut measurement = Measurement {
        seconds: Some(1),
        target: Some((2048, 256, 0)),
        window_policy: None,
        stage: Stage::Baseline,
    };
    let since = Instant::now() - Duration::from_millis(1001);
    measurement.stage = Stage::Collecting {
        since,
        generation: snapshot.generation.0,
        revision: snapshot.graph.candidate_revision,
        native: 7,
    };
    for _ in 0..2 {
        measurement
            .observe(&snapshot, &handle, &mut report, Instant::now(), 1000)
            .unwrap();
        assert!(
            matches!(measurement.stage, Stage::Collecting { since: anchor, .. } if anchor == since)
        );
    }
    if let Stage::Collecting { since, .. } = &mut measurement.stage {
        *since = Instant::now() - Duration::from_millis(2100);
    }
    assert!(
        measurement
            .observe(&snapshot, &handle, &mut report, Instant::now(), 1000)
            .unwrap_err()
            .to_string()
            .contains("startup deadline")
    );
    assert!(mailbox.try_request().is_none());
}

#[test]
fn window_cli_collection_submits_exact_window_request_instead_of_per_wake_precheck() {
    let (mut snapshot, mut report, _) = fixture();
    let node = snapshot
        .graph
        .node_of_kind(NodeKind::Source)
        .unwrap()
        .0
        .id();
    let mut observed = crate::service::measured_process::tests::observation(node);
    // The window evaluator belongs to the service. A missing legacy per-wake
    // summary must not silently select the old Apply route or bypass refusal.
    observed.consumer = None;
    let native = snapshot.native_observations.as_mut().unwrap();
    native.observation.owner_generation = observed.owner_generation;
    native.observation.process_delivery = vec![observed];
    let (handle, mailbox) =
        crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
    let mut measurement = Measurement::new(&window_options());
    measurement.stage = Stage::Collecting {
        since: Instant::now() - Duration::from_secs(2),
        generation: snapshot.generation.0,
        revision: snapshot.graph.candidate_revision,
        native: 7,
    };
    measurement
        .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
        .unwrap();
    let Some(crate::service::AppRequest::ApplyWindowedProcessBridge { request, policy }) =
        mailbox.try_request()
    else {
        panic!("window mode must enqueue the exact window Apply variant")
    };
    assert_eq!(request.node, node);
    assert_eq!(request.session_generation, snapshot.generation);
    assert_eq!(request.native_owner_generation, 7);
    assert_eq!(request.graph_revision, snapshot.graph.candidate_revision);
    assert_eq!(request.capacity_frames, 2048);
    assert_eq!(request.target_frames, 256);
    assert_eq!(request.jitter_frames, 0);
    assert_eq!(policy, window_policy());
    assert!(!measurement.ready());
}

#[test]
fn window_cli_omitted_policy_still_submits_the_legacy_measured_request() {
    let (mut snapshot, mut report, _) = fixture();
    let node = snapshot
        .graph
        .node_of_kind(NodeKind::Source)
        .unwrap()
        .0
        .id();
    let observed = crate::service::measured_process::tests::observation(node);
    let native = snapshot.native_observations.as_mut().unwrap();
    native.observation.owner_generation = observed.owner_generation;
    native.observation.process_delivery = vec![observed];
    let (handle, mailbox) =
        crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
    let mut options = MonitorOptions::default();
    for (flag, value) in [
        ("--measure-seconds", "1"),
        ("--target-frames", "256"),
        ("--capacity-frames", "2048"),
        ("--jitter-frames", "0"),
    ] {
        options.set(flag, value).unwrap();
    }
    let mut measurement = Measurement::new(&options);
    measurement.stage = Stage::Collecting {
        since: Instant::now() - Duration::from_secs(2),
        generation: snapshot.generation.0,
        revision: snapshot.graph.candidate_revision,
        native: 7,
    };
    measurement
        .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
        .unwrap();
    assert!(matches!(
        mailbox.try_request(),
        Some(crate::service::AppRequest::ApplyMeasuredProcessBridge(_))
    ));
}

#[test]
fn window_cli_confirmation_requires_exact_observed_policy_not_only_request() {
    for variant in 0..8 {
        let (mut snapshot, mut report, request) = fixture();
        let policy = window_policy();
        let mut active = snapshot.active_measured_process.take().unwrap();
        active.observed_window = Some(crate::service::MeasuredProcessWindow {
            policy,
            cutoff_100ns: 1_000_000,
            replayed_packet_events: 10,
            replayed_read_events: 8,
        });
        if variant == 0 {
            active.observed_window = None;
        } else if variant < 6 {
            let observed = &mut active.observed_window.as_mut().unwrap().policy;
            match variant {
                1 => observed.max_initial_unserved_frames += 1,
                2 => observed.max_initial_prime_discarded_frames += 1,
                3 => observed.min_output_frames += 1,
                4 => observed.min_output_reads += 1,
                5 => observed.min_output_span_100ns += 1,
                _ => unreachable!(),
            }
        } else if variant == 6 {
            active.request.target_frames += 1;
        }
        snapshot.active_measured_process = Some(active.clone());
        snapshot.measured_process = Some(active);
        let (handle, _) =
            crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
        let mut measurement = Measurement::new(&window_options());
        measurement.stage = Stage::Applying {
            since: Instant::now(),
            request,
            id: RequestId(0),
        };
        measurement
            .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
            .unwrap();
        assert_eq!(measurement.ready(), variant == 7, "variant={variant}");
    }
}

#[test]
fn window_cli_legacy_apply_cannot_confirm_a_window_selection_with_the_same_request() {
    let (mut snapshot, mut report, request) = fixture();
    let mut active = snapshot.active_measured_process.take().unwrap();
    active.observed_window = Some(crate::service::MeasuredProcessWindow {
        policy: window_policy(),
        cutoff_100ns: 1_000_000,
        replayed_packet_events: 10,
        replayed_read_events: 8,
    });
    snapshot.active_measured_process = Some(active.clone());
    snapshot.measured_process = Some(active);
    let (handle, _) =
        crate::service::ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
    let mut measurement = Measurement {
        seconds: Some(1),
        target: Some((2048, 256, 0)),
        window_policy: None,
        stage: Stage::Applying {
            since: Instant::now(),
            request,
            id: RequestId(0),
        },
    };
    measurement
        .observe(&snapshot, &handle, &mut report, Instant::now(), 30000)
        .unwrap();
    assert!(!measurement.ready());
}

#[test]
fn window_cli_ongoing_confirmation_cannot_accept_a_different_requested_policy() {
    let (mut snapshot, mut report, _) = fixture();
    report.requested_window_policy = Some(window_policy());
    let mut active = snapshot.active_measured_process.take().unwrap();
    active.observed_window = Some(crate::service::MeasuredProcessWindow {
        policy: window_policy(),
        cutoff_100ns: 1_000_000,
        replayed_packet_events: 10,
        replayed_read_events: 8,
    });
    snapshot.active_measured_process = Some(active.clone());
    report.confirmed_active_measured = Some(active);
    assert!(confirmed_run_matches(&snapshot, &report));
    for variant in 0..6 {
        report.requested_window_policy = Some(window_policy());
        if variant == 0 {
            report.requested_window_policy = None;
        } else {
            let requested = report.requested_window_policy.as_mut().unwrap();
            match variant {
                1 => requested.max_initial_unserved_frames += 1,
                2 => requested.max_initial_prime_discarded_frames += 1,
                3 => requested.min_output_frames += 1,
                4 => requested.min_output_reads += 1,
                5 => requested.min_output_span_100ns += 1,
                _ => unreachable!(),
            }
        }
        assert!(
            !confirmed_run_matches(&snapshot, &report),
            "variant={variant}"
        );
    }
}

#[test]
fn window_cli_failure_report_preserves_requested_policy_without_claiming_acceptance() {
    struct RefusingDriver;
    impl ServiceDriver for RefusingDriver {
        fn request(
            &mut self,
            context: &mut ServiceContext,
            _: &mut crate::service::JobPool,
            request: crate::service::AppRequest,
        ) {
            if matches!(request, crate::service::AppRequest::Start(_)) {
                context.graph.record_error("injected baseline refusal");
            }
        }
        fn is_drained(&self) -> bool {
            true
        }
    }
    let mut options = window_options();
    options.pid = Some(42);
    options.output = Some("pinned-output".into());
    let report = run_with_driver(
        options,
        InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        },
        RefusingDriver,
        Duration::from_millis(1),
    )
    .unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["requested_window_policy"],
        serde_json::to_value(window_policy()).unwrap()
    );
    assert!(report.confirmed_active_measured.is_none());
    assert!(!report.qualified);
    assert!(!report.run_complete);
    assert!(report.cleanup_complete);
    assert!(
        report
            .failure
            .as_deref()
            .unwrap()
            .contains("baseline refusal")
    );
}
