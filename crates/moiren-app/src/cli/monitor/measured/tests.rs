use super::*;

#[test]
fn measured_collection_has_a_finite_unrenewed_observation_deadline() {
    let mut measurement = Measurement {
        seconds: Some(1),
        target: Some((2048, 256, 0)),
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
