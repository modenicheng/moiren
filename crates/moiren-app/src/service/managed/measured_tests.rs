use super::*;
use crate::service::measured_process::tests::observation;
use crate::service::{MeasuredProcessRequest, NativeSessionObservations};
use moiren_windows_audio::engine_backend::NativeObservationSnapshot;

pub(super) fn start_process(r: &mut Rig) -> MeasuredProcessRequest {
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
            && r.driver.confirm_parameters.is_none()
    });
    let node = r.node(NodeKind::Source);
    let generation = r
        .driver
        .owner
        .as_ref()
        .unwrap()
        .observer
        .read()
        .state
        .owner_generation;
    let mut observation = observation(node);
    observation.owner_generation = generation;
    let request = MeasuredProcessRequest {
        node,
        session_generation: r.c.core.generation(),
        native_owner_generation: generation,
        graph_revision: r.c.graph.snapshot().candidate_revision,
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    };
    r.c.native_observations = Some(NativeSessionObservations {
        generation: request.session_generation,
        observation: NativeObservationSnapshot {
            virtual_clock: None,
            owner_generation: generation,
            running: true,
            streams: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
            render_padding: Vec::new(),
            process_delivery: vec![observation],
        },
    });
    request
}

#[test]
fn measured_apply_replaces_owner_and_reaches_both_precision_factories() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (f32_backend, f64_backend) =
            (FakeBackend(r.state.clone()), FakeBackend(r.state.clone()));
        let first = observed.clone();
        r.driver.factory = Arc::new(move |_, _, _, measured| {
            first.lock().unwrap().push((
                ProcessingPrecision::F32,
                measured.map(|p| p.summary().clone()),
            ));
            Ok(Box::new(f32_backend.clone()))
        });
        let second = observed.clone();
        r.driver.f64_factory = Some(Arc::new(move |_, _, _, measured| {
            second.lock().unwrap().push((
                ProcessingPrecision::F64,
                measured.map(|p| p.summary().clone()),
            ));
            Ok(Box::new(f64_backend.clone()))
        }));
        let request = start_process(&mut r);
        r.request(AppRequest::ApplyMeasuredProcessBridge(request));
        let requested = r.c.measured_process.as_ref().unwrap().summary().clone();
        assert!(r.c.active_measured_process.is_none());
        r.wait(|r| r.c.active_measured_process.as_ref() == Some(&requested));
        assert_ne!(r.c.core.generation(), request.session_generation);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
        assert_eq!(
            *observed.lock().unwrap(),
            vec![(precision, None), (precision, Some(requested.clone()))]
        );
        r.request(AppRequest::SetTransportLatency(Default::default()));
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.c.active_measured_process, Some(requested));
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| {
            r.c.core.phase() == SessionPhase::Running && r.c.active_measured_process.is_none()
        });
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 3);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn stale_or_invalid_measured_selection_preserves_running_owner() {
    for fault in 0..6 {
        let mut r = Rig::new();
        let mut request = start_process(&mut r);
        match fault {
            0 => request.session_generation.0 += 1,
            1 => request.native_owner_generation += 1,
            2 => request.graph_revision += 1,
            3 => request.target_frames = 128,
            4 => {
                r.c.native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .process_delivery[0]
                    .process
                    .creation_time_100ns += 1
            }
            5 => {
                r.c.native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .process_delivery[0]
                    .consumer = None
            }
            _ => unreachable!(),
        }
        r.request(AppRequest::ApplyMeasuredProcessBridge(request));
        assert!(r.c.measured_process.is_none());
        assert!(r.c.graph.snapshot().last_error.is_some(), "fault {fault}");
        assert_eq!(r.c.core.phase(), SessionPhase::Running);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn ingress_elapsed_desired_or_active_policy_refuses_measured_apply_before_side_effects() {
    for active in [false, true] {
        let mut r = Rig::new();
        let request = start_process(&mut r);
        if active {
            r.c.native_observations
                .as_mut()
                .unwrap()
                .observation
                .process_delivery[0]
                .admitted_bridge
                .max_ingress_elapsed_100ns = Some(50_000);
        } else {
            // Keep the captured owner settings matching: this is a supported
            // baseline policy, not a stale-settings refusal.
            r.c.transport_latency.process_max_ingress_elapsed_100ns = Some(50_000);
            r.driver.owner.as_mut().unwrap().transport_latency = r.c.transport_latency;
            r.c.active_transport_latency = Some(r.c.transport_latency);
        }
        let selection = r.driver.measured_selection_id;
        let captured = r.driver.apply_settings.as_ref().map(|settings| {
            (
                settings.audio.clone(),
                settings.transport,
                settings.profile,
                settings
                    .measured_process
                    .as_ref()
                    .map(|profile| profile.summary().clone()),
            )
        });
        r.request(AppRequest::ApplyMeasuredProcessBridge(request));
        assert!(
            r.c.graph
                .snapshot()
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("ingress"))
        );
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.driver.measured_selection_id, selection);
        assert!(!r.driver.apply);
        assert_eq!(
            r.driver.apply_settings.as_ref().map(|settings| {
                (
                    settings.audio.clone(),
                    settings.transport,
                    settings.profile,
                    settings
                        .measured_process
                        .as_ref()
                        .map(|profile| profile.summary().clone()),
                )
            }),
            captured,
            "refusal must preserve the previous captured configuration"
        );
        assert_eq!(r.c.core.phase(), SessionPhase::Running);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn later_transport_draft_does_not_mutate_a_captured_measured_replacement() {
    let mut r = Rig::new();
    let backend = FakeBackend(r.state.clone());
    r.driver.factory = Arc::new(move |_, _, _, _| Ok(Box::new(backend.clone())));
    let request = start_process(&mut r);
    r.request(AppRequest::ApplyMeasuredProcessBridge(request));
    let requested = r.c.measured_process.as_ref().unwrap().summary().clone();
    r.request(AppRequest::SetTransportLatency(Default::default()));
    assert!(r.c.measured_process.is_none());
    r.wait(|r| r.c.active_measured_process.as_ref() == Some(&requested));
    assert!(crate::service::snapshot(&r.c).measured_process.is_none());
    assert_eq!(
        crate::service::snapshot(&r.c).active_measured_process,
        Some(requested)
    );
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
}

#[test]
fn graph_edit_after_measured_selection_requires_fresh_selection_before_retirement() {
    let mut r = Rig::new();
    r.driver
        .set_measured_test_factory(FakeBackend(r.state.clone()));
    let request = start_process(&mut r);
    r.request(AppRequest::ApplyMeasuredProcessBridge(request));
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.tick();
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
    assert!(r.c.active_measured_process.is_none());
    assert!(
        r.c.graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("graph revision"))
    );
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
    assert!(r.c.measured_process.is_none());
}

#[test]
fn measured_backend_refusal_retains_desired_profile_without_active_claim() {
    let mut r = Rig::new();
    let request = start_process(&mut r);
    r.driver.factory = Arc::new(|_, _, _, measured| {
        assert!(measured.is_some());
        Err(host::BackendError::Unsupported(
            "injected actual demand refusal".into(),
        ))
    });
    r.request(AppRequest::ApplyMeasuredProcessBridge(request));
    let desired = r.c.measured_process.as_ref().unwrap().summary().clone();
    r.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    assert_eq!(
        r.c.measured_process
            .as_ref()
            .map(|profile| profile.summary()),
        Some(&desired)
    );
    assert!(
        crate::service::snapshot(&r.c)
            .active_measured_process
            .is_none()
    );
    assert!(
        r.c.graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("actual demand refusal"))
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
}

fn assert_cleanup_refused_replacement(r: &mut Rig) {
    assert!(
        r.driver.owner.is_none(),
        "failed predecessor cleanup must not admit a replacement owner"
    );
    assert_eq!(r.c.core.phase(), SessionPhase::Failed);
    assert!(
        r.c.core
            .error()
            .is_some_and(|error| error.contains("cleanup"))
    );
    assert!(
        r.c.graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("cleanup"))
    );
    let snapshot = crate::service::snapshot(&r.c);
    assert!(snapshot.active_audio_settings.is_none());
    assert!(snapshot.active_transport_latency.is_none());
    assert!(snapshot.active_measured_process.is_none());
    assert!(!r.driver.apply);
    assert!(r.driver.apply_settings.is_none());
    for _ in 0..5 {
        r.tick();
    }
    assert_eq!(r.state.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn measured_replacement_refuses_failed_predecessor_cleanup() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        let request = start_process(&mut r);
        r.state.fail_stop.store(true, Ordering::SeqCst);
        r.request(AppRequest::ApplyMeasuredProcessBridge(request));
        let desired = r.c.measured_process.as_ref().unwrap().summary().clone();
        r.wait(|r| r.c.last_cleanup.is_some());
        let cleanup = r.c.last_cleanup.as_ref().unwrap();
        assert_eq!(cleanup.generation, request.session_generation);
        assert_eq!(cleanup.state.host, host::HostState::Closed);
        assert!(
            cleanup
                .state
                .cleanup_error
                .as_ref()
                .is_some_and(|error| error.to_string().contains("injected stop failure"))
        );
        assert_cleanup_refused_replacement(&mut r);
        assert_eq!(
            r.c.measured_process
                .as_ref()
                .map(|profile| profile.summary()),
            Some(&desired)
        );
        // A later Apply cannot bypass unresolved cleanup by creating another host.
        r.request(AppRequest::ApplyGraph);
        r.tick();
        assert_cleanup_refused_replacement(&mut r);
        assert_eq!(
            r.c.measured_process
                .as_ref()
                .map(|profile| profile.summary()),
            Some(&desired)
        );
    }
}

#[test]
fn declared_replacement_refuses_failed_predecessor_cleanup() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio.clone()));
        r.start();
        let generation = r.c.core.generation();
        audio.processing.block_frames = 512;
        r.request(AppRequest::SetAudioSettings(audio.clone()));
        r.state.fail_stop.store(true, Ordering::SeqCst);
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| r.c.last_cleanup.is_some());
        assert_eq!(r.c.last_cleanup.as_ref().unwrap().generation, generation);
        assert_cleanup_refused_replacement(&mut r);
        assert_eq!(r.c.audio_settings, audio);
        let receipt = r.c.last_cleanup.clone().unwrap();
        r.state.fail_stop.store(false, Ordering::SeqCst);
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| r.c.core.phase() == SessionPhase::Running);
        assert_eq!(
            r.c.last_cleanup.as_ref().unwrap().generation,
            receipt.generation
        );
        assert_eq!(r.c.last_cleanup.as_ref().unwrap().state, receipt.state);
        assert_eq!(r.state.prepares.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn explicit_declared_retry_after_closed_cleanup_preserves_failure_receipt() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        for start in [false, true] {
            let mut r = Rig::new();
            r.driver
                .set_measured_test_factory(FakeBackend(r.state.clone()));
            let mut audio = AudioSettings::default();
            audio.processing.precision = precision;
            r.request(AppRequest::SetAudioSettings(audio));
            let request = start_process(&mut r);
            r.state.fail_stop.store(true, Ordering::SeqCst);
            r.request(AppRequest::ApplyMeasuredProcessBridge(request));
            r.wait(|r| r.c.last_cleanup.is_some());
            assert_cleanup_refused_replacement(&mut r);
            let receipt = r.c.last_cleanup.clone().unwrap();
            // Declared retry must be explicit and must not silently reuse the
            // measured evidence admitted against the retired native owner.
            r.request(AppRequest::SetTransportLatency(Default::default()));
            r.state.fail_stop.store(false, Ordering::SeqCst);
            if start {
                r.start();
            } else {
                r.request(AppRequest::ApplyGraph);
                r.wait(|r| r.c.core.phase() == SessionPhase::Running);
            }
            assert_eq!(
                r.c.last_cleanup.as_ref().unwrap().generation,
                receipt.generation
            );
            assert_eq!(r.c.last_cleanup.as_ref().unwrap().state, receipt.state);
            assert!(r.c.measured_process.is_none());
            assert!(r.c.active_measured_process.is_none());
            assert_eq!(r.state.prepares.load(Ordering::SeqCst), 2);
            assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
            assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
            r.request(AppRequest::Stop);
            r.wait(|r| r.driver.is_drained());
        }
    }
}

#[test]
fn declared_replacement_allows_recoverable_predecessor_disconnect() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio.clone()));
        r.start();
        r.state.fail_owner.store(true, Ordering::SeqCst);
        r.wait(|r| {
            r.c.core.phase() == SessionPhase::Failed
                && r.driver
                    .owner
                    .as_ref()
                    .unwrap()
                    .observer
                    .read()
                    .state
                    .execution
                    == ExecutionState::WaitingForResources
        });
        r.state.fail_owner.store(false, Ordering::SeqCst);
        audio.processing.block_frames = 512;
        r.request(AppRequest::SetAudioSettings(audio));
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| r.c.core.phase() == SessionPhase::Running);
        let cleanup = r.c.last_cleanup.as_ref().unwrap();
        assert_eq!(cleanup.state.host, host::HostState::Closed);
        assert!(matches!(
            cleanup.state.cleanup_error,
            Some(host::BackendError::ResourceUnavailable(_))
        ));
        assert!(cleanup.state.fault.is_none());
        assert_eq!(r.state.prepares.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn measured_replacement_refuses_failed_predecessor_cleanup_receipts() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        for (host_state, cleanup_error, fault) in [
            (host::HostState::Opening, None, None),
            (host::HostState::Open, None, None),
            (host::HostState::Closing, None, None),
            (
                host::HostState::Closed,
                None,
                Some(host::Diagnostic::Fault("injected final fault".into())),
            ),
            (
                host::HostState::Closed,
                Some(host::BackendError::ResourceUnavailable(
                    "owner disconnected".into(),
                )),
                None,
            ),
        ] {
            let mut r = Rig::new();
            r.driver
                .set_measured_test_factory(FakeBackend(r.state.clone()));
            let mut audio = AudioSettings::default();
            audio.processing.precision = precision;
            r.request(AppRequest::SetAudioSettings(audio));
            let request = start_process(&mut r);
            r.request(AppRequest::ApplyMeasuredProcessBridge(request));
            let desired = r.c.measured_process.as_ref().unwrap().summary().clone();
            // Run the real host Drop/join, holding its completion before the
            // service consumes it so each invalid final-state shape is isolated.
            let budget = RoundBudget {
                completions: 0,
                ..RoundBudget::default()
            };
            r.tick_with_budget(budget);
            assert!(r.driver.owner.is_none());
            assert_eq!(r.driver.cleaning, 1);
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut cleanup = loop {
                if let Some(completion) = r.driver.queue().try_recv() {
                    let Completion::Reaped(cleanup) = completion else {
                        panic!("expected predecessor cleanup completion");
                    };
                    break cleanup;
                }
                assert!(
                    Instant::now() < deadline,
                    "predecessor cleanup did not finish"
                );
                r.tick_with_budget(budget);
            };
            assert_eq!(cleanup.generation, request.session_generation);
            assert_eq!(cleanup.state.host, host::HostState::Closed);
            assert!(cleanup.state.cleanup_error.is_none());
            assert!(cleanup.state.fault.is_none());
            cleanup.state.host = host_state;
            cleanup.state.cleanup_error = cleanup_error;
            cleanup.state.fault = fault;
            let expected = cleanup.state.clone();
            r.driver
                .queue()
                .port()
                .try_reserve()
                .unwrap()
                .send_or_dispose(Completion::Reaped(cleanup), drop);
            r.tick();
            assert_cleanup_refused_replacement(&mut r);
            assert_eq!(r.c.last_cleanup.as_ref().unwrap().state, expected);
            assert_eq!(
                r.c.measured_process
                    .as_ref()
                    .map(|profile| profile.summary()),
                Some(&desired)
            );
            r.request(AppRequest::SetTransportLatency(Default::default()));
            r.request(AppRequest::ApplyGraph);
            if host_state == host::HostState::Closed {
                r.wait(|r| r.c.core.phase() == SessionPhase::Running);
                assert_eq!(r.c.last_cleanup.as_ref().unwrap().state, expected);
                assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
                assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
                r.request(AppRequest::Stop);
                r.wait(|r| r.driver.is_drained());
            } else {
                r.tick();
                assert_cleanup_refused_replacement(&mut r);
            }
        }
    }
}
