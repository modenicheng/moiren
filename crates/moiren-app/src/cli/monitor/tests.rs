use super::*;
use crate::service::{
    ManagedDriver, ProcessingPrecision,
    test_backend::{FakeBackend, State},
};
use std::sync::{Arc, atomic::Ordering};

fn options() -> MonitorOptions {
    let mut options = MonitorOptions::default();
    options.set("--process", "4294967294").unwrap();
    options.set("--output", "pinned-output").unwrap();
    options
}

fn source() -> InputSelection {
    InputSelection::Process {
        pid: 4294967294,
        creation_time_100ns: 12345,
    }
}

#[test]
fn actual_context_is_only_io_bus_with_stable_process_and_pinned_output() {
    let (context, spec) = context(&options(), source()).unwrap();
    let graph = context.graph.snapshot();
    assert_eq!(
        graph
            .graph
            .nodes()
            .iter()
            .map(|node| node.kind())
            .collect::<Vec<_>>(),
        vec![NodeKind::Source, NodeKind::Bus, NodeKind::Sink]
    );
    assert_eq!(graph.graph.edges().len(), 2);
    assert_eq!(spec.source, source());
    assert_eq!(spec.output_endpoint_id, "pinned-output");
    assert_eq!(
        graph.node_of_kind(NodeKind::Source).unwrap().1.source,
        Some(source())
    );
    assert_eq!(
        context.transport_latency,
        TransportLatencySettings::default()
    );
    assert!(context.loaded_project.is_none());
    assert!(context.graph.snapshot().exports.is_empty());
}

#[test]
fn context_refuses_missing_or_mismatched_stable_identity_before_start() {
    for selection in [
        InputSelection::Process {
            pid: 4294967294,
            creation_time_100ns: 0,
        },
        InputSelection::Process {
            pid: 12,
            creation_time_100ns: 12345,
        },
        InputSelection::Tone {
            frequency_hz: 440.0,
        },
    ] {
        assert!(context(&options(), selection).is_err());
    }
}

#[test]
fn supervised_real_service_confirms_both_precisions_and_reaps_owner() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let state = Arc::new(State::default());
        let f32_backend = FakeBackend(state.clone());
        let f64_backend = f32_backend.clone();
        let driver = ManagedDriver::new(move |_| Ok(Box::new(f32_backend.clone())))
            .with_f64_factory(move |_| Ok(Box::new(f64_backend.clone())));
        let mut options = options();
        options.audio.processing.precision = precision;
        let report = run_with_driver(options, source(), driver, Duration::from_millis(20)).unwrap();
        assert!(report.run_complete, "{report:?}");
        assert!(report.cleanup_complete, "{report:?}");
        assert_eq!(report.failure, None);
        assert!(!report.qualified);
        assert!(
            serde_json::to_value(&report)
                .unwrap()
                .get("final_after_owner_join")
                .is_some()
        );
        assert_eq!(
            serde_json::to_value(&report).unwrap()["final_after_owner_join"],
            serde_json::Value::Null,
            "a fake backend without retained native evidence must stay unknown"
        );
        assert_eq!(
            report.confirmed_active_audio.unwrap().processing.precision,
            precision
        );
        assert_eq!(
            report.confirmed_active_transport,
            Some(TransportLatencySettings::default())
        );
        assert_eq!(state.starts.load(Ordering::SeqCst), 1);
        assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    }
}

#[cfg(windows)]
fn joined_fixture() -> (MonitorReport, crate::service::SessionCleanupObservation) {
    use moiren_engine::host::*;
    use moiren_windows_audio::{
        engine_backend::{
            NativeObservationSnapshot, ProfessionalObservationSnapshot,
            ProfessionalStreamObservation,
        },
        native::NativeStreamInfo,
    };
    let backend = FakeBackend::default();
    let mut report = run_with_driver(
        options(),
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    report.confirmed_generation = Some(2);
    report.confirmed_native_generation = Some(1);
    let native = NativeObservationSnapshot {
        owner_generation: 1,
        running: false,
        streams: Vec::new(),
        bridges: Vec::new(),
        boundary_probes: Vec::new(),
        render_padding: Vec::new(),
        process_delivery: Vec::new(),
        virtual_clock: None,
    };
    let mut graph = LogicalGraph::default();
    let node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let backend = ProfessionalObservationSnapshot {
        owner_generation: 1,
        running: false,
        streams: vec![ProfessionalStreamObservation {
            node,
            input: false,
            master: true,
            requested: Default::default(),
            actual: NativeStreamInfo {
                sample_rate: 48000,
                format: crate::service::SampleFormat::Float32,
                channels: 2,
                buffer_frames: 256,
            },
            dropped_frames: 3,
            silent_frames: 5,
            native_callbacks: 7,
            native_wait_wakes: 11,
            native_wait_timeouts: 13,
            native_written_frames: 17024,
            native_dropped_frames: 17,
            native_silent_frames: 19,
            native_capture_quality: None,
            driver_latency: None,
        }],
    };
    let cleanup = crate::service::SessionCleanupObservation {
        generation: crate::service::SessionGeneration(2),
        state: HostSnapshot {
            host: HostState::Closed,
            execution: ExecutionState::Dormant,
            desired_revision: 1,
            active_revision: None,
            plan_revision: None,
            owner_generation: 1,
            resource_generation: 1,
            timeline: None,
            update: UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        },
        process_trace: None,
        final_observations: Some(Arc::new(crate::service::JoinedSessionObservations {
            graph_revision: report.confirmed_revision.unwrap(),
            host_revision: 1,
            native: Some(native),
            backend: Some(backend),
        })),
    };
    (report, cleanup)
}

#[cfg(windows)]
#[test]
fn joined_tally_serializes_exact_counters_and_identity_without_relabelling_live_sample() {
    let (mut report, cleanup) = joined_fixture();
    let mut running = cleanup
        .final_observations
        .as_ref()
        .unwrap()
        .native
        .clone()
        .unwrap();
    running.running = true;
    report.last_running_native = Some(running.clone());
    report.capture_joined_tally(&cleanup);
    let json = serde_json::to_value(&report).unwrap();
    let tally = &json["final_after_owner_join"];
    assert_eq!(tally["observation_scope"], "final_after_owner_join");
    assert_eq!(tally["app_generation"], 2);
    assert_eq!(tally["native_owner_generation"], 1);
    assert_eq!(tally["graph_revision"], report.confirmed_revision.unwrap());
    assert_eq!(tally["host_revision"], 1);
    assert_eq!(tally["native"]["running"], false);
    let stream = &tally["backend"]["streams"][0];
    for (key, value) in [
        ("native_written_frames", 17024),
        ("native_wait_wakes", 11),
        ("native_wait_timeouts", 13),
        ("native_callbacks", 7),
        ("dropped_frames", 3),
        ("silent_frames", 5),
        ("native_dropped_frames", 17),
        ("native_silent_frames", 19),
    ] {
        assert_eq!(stream[key], value, "{key}");
    }
    assert!(stream["native_capture_quality"].is_null());
    assert!(stream["driver_latency"].is_null());
    assert_eq!(report.last_running_native, Some(running));
    assert!(!report.qualified);
}

#[cfg(windows)]
#[test]
fn joined_tally_rejects_stale_identity_or_still_running_evidence() {
    for field in 0..9 {
        let (mut report, mut cleanup) = joined_fixture();
        let joined = Arc::make_mut(cleanup.final_observations.as_mut().unwrap());
        match field {
            0 => cleanup.generation.0 += 1,
            1 => cleanup.state.owner_generation += 1,
            2 => joined.graph_revision += 1,
            3 => joined.host_revision += 1,
            4 => joined.native.as_mut().unwrap().owner_generation += 1,
            5 => joined.backend.as_mut().unwrap().owner_generation += 1,
            6 => joined.native.as_mut().unwrap().running = true,
            7 => joined.backend.as_mut().unwrap().running = true,
            8 => cleanup.state.host = moiren_engine::host::HostState::Open,
            _ => unreachable!(),
        }
        report.capture_joined_tally(&cleanup);
        assert!(report.final_after_owner_join.is_none(), "field {field}");
        assert!(!report.cleanup_complete, "field {field}");
        assert!(
            report
                .failure
                .unwrap()
                .contains("identity or closure mismatch")
        );
    }
}

#[cfg(windows)]
#[test]
fn joined_tally_preserves_failure_and_unknown_counters_without_live_fallback() {
    let (mut report, mut cleanup) = joined_fixture();
    report.failure = Some("earlier terminal failure".into());
    cleanup.state.cleanup_error = Some(moiren_engine::host::BackendError::Failed(
        "late stop error".into(),
    ));
    report.capture_joined_tally(&cleanup);
    assert_eq!(report.failure.as_deref(), Some("earlier terminal failure"));
    assert!(
        report.final_after_owner_join.as_ref().unwrap()["cleanup_error"]
            .as_str()
            .unwrap()
            .contains("late stop error")
    );
    let joined = Arc::make_mut(cleanup.final_observations.as_mut().unwrap());
    report.last_running_native = joined.native.clone();
    joined.native = None;
    report.capture_joined_tally(&cleanup);
    assert!(report.final_after_owner_join.as_ref().unwrap()["native"].is_null());
    Arc::make_mut(cleanup.final_observations.as_mut().unwrap()).backend = None;
    report.capture_joined_tally(&cleanup);
    assert!(report.final_after_owner_join.is_none());
    cleanup.final_observations = None;
    report.capture_joined_tally(&cleanup);
    assert!(report.final_after_owner_join.is_none());
    assert!(report.last_running_native.is_some());
}

#[test]
fn preparation_refusal_is_not_active_or_successful_but_still_joins() {
    let driver = ManagedDriver::new(|_| {
        Err(moiren_engine::host::BackendError::Unsupported(
            "refused test budget".into(),
        ))
    });
    let report = run_with_driver(options(), source(), driver, Duration::from_millis(20)).unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete);
    assert!(report.confirmed_active_audio.is_none());
    assert!(report.failure.unwrap().contains("refused test budget"));
}

#[test]
fn owner_fault_keeps_failure_and_reaps_without_claiming_run_complete() {
    let state = Arc::new(State::default());
    state.fail_owner.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state.clone());
    let report = run_with_driver(
        options(),
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(!report.run_complete);
    // OwnerReturn carries this injected error through cleanup as well. A
    // successful service join must not erase that final owner error.
    assert!(!report.cleanup_complete, "{report:?}");
    assert!(report.failure.is_some());
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn unavailable_resource_fails_without_claiming_an_active_session() {
    let state = Arc::new(State::default());
    state.unavailable.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state);
    let mut options = options();
    options.startup_timeout_ms = 30;
    let report = run_with_driver(
        options,
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete);
    assert!(report.failure.is_some(), "{report:?}");
    assert!(report.confirmed_generation.is_none());
}

#[test]
fn startup_deadline_exits_and_joins_when_no_owner_is_ever_confirmed() {
    struct WaitingDriver;
    impl ServiceDriver for WaitingDriver {
        fn request(
            &mut self,
            context: &mut ServiceContext,
            _: &mut crate::service::JobPool,
            request: crate::service::AppRequest,
        ) {
            if let crate::service::AppRequest::Start(spec) = request {
                context.core.start(spec).unwrap();
            }
        }
        fn is_drained(&self) -> bool {
            true
        }
    }
    let mut options = options();
    options.startup_timeout_ms = 30;
    let report =
        run_with_driver(options, source(), WaitingDriver, Duration::from_millis(20)).unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete, "{report:?}");
    assert!(report.failure.unwrap().contains("startup deadline"));
}

#[test]
fn cleanup_failure_is_retained_even_after_the_run_duration_completes() {
    let state = Arc::new(State::default());
    state.fail_stop.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state.clone());
    let report = run_with_driver(
        options(),
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(report.run_complete, "{report:?}");
    assert!(!report.cleanup_complete, "{report:?}");
    assert!(
        report
            .failure
            .as_deref()
            .is_some_and(|error| error.contains("injected stop failure")),
        "{report:?}"
    );
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}

#[cfg(windows)]
struct MeasuredDriver {
    inner: ManagedDriver,
}

#[cfg(windows)]
impl ServiceDriver for MeasuredDriver {
    fn request(
        &mut self,
        c: &mut ServiceContext,
        pool: &mut crate::service::JobPool,
        request: crate::service::AppRequest,
    ) {
        self.inner.request(c, pool, request);
    }
    fn poll(
        &mut self,
        c: &mut ServiceContext,
        pool: &mut crate::service::JobPool,
        budget: crate::service::RoundBudget,
    ) {
        self.inner.poll(c, pool, budget);
        if c.core.phase() == SessionPhase::Running {
            let graph = c.graph.snapshot();
            let node = graph.node_of_kind(NodeKind::Source).unwrap().0.id();
            let mut observed = crate::service::measured_process::tests::observation(node);
            // A newly created EngineHost starts at native owner generation 1.
            observed.owner_generation = 1;
            observed.process.pid = 4294967294;
            observed.process.creation_time_100ns = 12345;
            c.native_observations = Some(crate::service::NativeSessionObservations {
                generation: c.core.generation(),
                observation: moiren_windows_audio::engine_backend::NativeObservationSnapshot {
                    virtual_clock: None,
                    owner_generation: 1,
                    running: true,
                    streams: Vec::new(),
                    bridges: Vec::new(),
                    boundary_probes: Vec::new(),
                    render_padding: Vec::new(),
                    process_delivery: vec![observed],
                },
            });
            let fresh = c.native_observations.as_ref().unwrap().observation.clone();
            self.inner
                .set_measured_snapshot_reader_for_test(move || Some(fresh.clone()));
        }
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, pool: &mut crate::service::JobPool) {
        self.inner.begin_exit(c, pool);
    }
    fn is_drained(&self) -> bool {
        self.inner.is_drained()
    }
}

#[cfg(windows)]
#[test]
fn measured_monitor_runs_baseline_then_applies_before_timing_requested_run() {
    let state = Arc::new(State::default());
    let f32_backend = FakeBackend(state.clone());
    let mut driver = ManagedDriver::new(move |_| Ok(Box::new(f32_backend.clone())));
    let selected_backend = FakeBackend(state.clone());
    driver.set_measured_test_factory(selected_backend);
    let mut options = options();
    for (flag, value) in [
        ("--measure-seconds", "1"),
        ("--target-frames", "256"),
        ("--capacity-frames", "2048"),
        ("--jitter-frames", "0"),
    ] {
        options.set(flag, value).unwrap();
    }
    let report = run_with_driver(
        options,
        source(),
        MeasuredDriver { inner: driver },
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(report.run_complete, "{report:?}");
    assert!(report.cleanup_complete, "{report:?}");
    assert_eq!(report.failure, None);
    assert!(!report.qualified);
    let measured = report.confirmed_active_measured.as_ref().unwrap();
    assert_eq!(
        measured.request.session_generation.0,
        report.measurement_generation.unwrap()
    );
    assert_ne!(report.confirmed_generation, report.measurement_generation);
    assert_eq!(measured.request.target_frames, 256);
    assert!(report.observed_measurement_ms >= 1000);
    assert!(report.observed_running_ms >= 20);
    assert_eq!(state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(state.stops.load(Ordering::SeqCst), 2);
}

#[cfg(windows)]
#[test]
fn measured_monitor_refuses_unadmitted_target_and_joins_only_baseline_owner() {
    let state = Arc::new(State::default());
    let backend = FakeBackend(state.clone());
    let mut driver = ManagedDriver::new(move |_| Ok(Box::new(backend.clone())));
    driver.set_measured_test_factory(FakeBackend(state.clone()));
    let mut options = options();
    for (flag, value) in [
        ("--measure-seconds", "1"),
        ("--target-frames", "128"),
        ("--capacity-frames", "2048"),
        ("--jitter-frames", "0"),
    ] {
        options.set(flag, value).unwrap();
    }
    let report = run_with_driver(
        options,
        source(),
        MeasuredDriver { inner: driver },
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete, "{report:?}");
    assert!(report.confirmed_active_measured.is_none());
    assert!(report.confirmed_generation.is_none());
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["requested_measured_target"]["capacity_frames"], 2048);
    assert_eq!(json["requested_measured_target"]["target_frames"], 128);
    assert_eq!(json["requested_measured_target"]["jitter_frames"], 0);
    assert!(
        report
            .failure
            .as_deref()
            .is_some_and(|error| error.contains("input target cannot cover")),
        "{report:?}"
    );
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}
