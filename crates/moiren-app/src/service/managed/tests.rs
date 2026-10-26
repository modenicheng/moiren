use super::*;
use crate::service::test_backend::{FakeBackend, State};
use moiren_core::graph::NodeKind;
use std::sync::atomic::Ordering;

struct Rig {
    driver: ManagedDriver,
    c: ServiceContext,
    pool: JobPool,
    state: Arc<State>,
}

#[test]
fn virtual_only_apply_runs_without_physical_session_selectors() {
    let mut rig = Rig::new();
    rig.driver.exports_runtime.adapter = Some(Arc::new(
        super::super::exports::runtime::tests::FakeAdapter::default(),
    ));
    let mut snapshot = rig.c.graph.snapshot();
    for node in &mut snapshot.nodes {
        node.source = None;
        node.output_endpoint_id = None;
    }
    let exports = [
        (ExportDirection::Playback, rig.node(NodeKind::Source)),
        (ExportDirection::Microphone, rig.node(NodeKind::Sink)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (direction, node))| ExportIntent {
        endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([index as u8 + 1; 16]),
        direction,
        name: format!("Virtual integration {index}"),
        channels: 2,
        enabled: true,
        binding: Some(node),
    })
    .collect();
    rig.c
        .graph
        .replace_with_exports(snapshot.graph, snapshot.nodes, exports)
        .unwrap();
    rig.request(AppRequest::ApplyGraph);
    for _ in 0..250 {
        rig.tick();
        if rig.c.core.phase() == SessionPhase::Running {
            break;
        }
    }
    assert_eq!(
        rig.c.core.phase(),
        SessionPhase::Running,
        "virtual graph rejected: {:?}",
        rig.c.graph.snapshot().last_error
    );
    rig.wait(|r| {
        r.c.exports
            .statuses(&r.c.graph.snapshot())
            .iter()
            .all(|status| status.gate_open)
    });
    assert!(
        rig.c.core.desired().is_none(),
        "no invented legacy selectors"
    );
    let first_generation = rig.c.core.generation();
    let mut settings = AudioSettings::default();
    settings.processing.block_frames = 512;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.core.generation() != first_generation
            && r.c.graph.snapshot().active_revision.is_some()
    });
    assert!(rig.c.core.desired().is_none());
    rig.request(AppRequest::Stop);
    rig.wait(|r| r.driver.cleaning == 0 && r.c.core.phase() == SessionPhase::Idle);
    assert!(
        rig.c
            .exports
            .statuses(&rig.c.graph.snapshot())
            .iter()
            .all(|status| { status.resource == ExportResourceState::Present && !status.gate_open })
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
}

#[test]
fn virtual_to_physical_apply_preserves_explicit_run_limit() {
    let (mut rig, _) = export_rig();
    rig.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::Seconds(12),
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    assert!(rig.c.core.desired().is_none());
    assert_eq!(rig.c.core.run_limit(), Some(RunLimit::Seconds(12)));
    let generation = rig.c.core.generation();
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(Vec::new())));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running && r.c.core.generation() != generation);
    assert_eq!(rig.c.core.run_limit(), Some(RunLimit::Seconds(12)));
    rig.driver.owner.as_mut().unwrap().started = Some(Instant::now() - Duration::from_secs(13));
    rig.tick();
    rig.wait(|r| r.c.core.phase() == SessionPhase::Idle && r.driver.cleaning == 0);
}

fn export_rig() -> (Rig, Arc<super::super::exports::runtime::tests::FakeAdapter>) {
    let mut rig = Rig::new();
    let adapter = Arc::new(super::super::exports::runtime::tests::FakeAdapter::default());
    rig.driver.exports_runtime.adapter = Some(adapter.clone());
    let export = ExportIntent {
        endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x91; 16]),
        direction: ExportDirection::Playback,
        name: "User named playback".into(),
        channels: 2,
        enabled: true,
        binding: Some(rig.node(NodeKind::Source)),
    };
    rig.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: rig.node(NodeKind::Sink),
        endpoint_id: "output".into(),
    }));
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![export])));
    (rig, adapter)
}

#[test]
fn export_presence_does_not_open_gate_until_engine_confirms_and_stop_retains_it() {
    let (mut rig, adapter) = export_rig();
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Present
    });
    assert!(!rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
    assert_eq!(rig.c.graph.snapshot().active_revision, None);
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(kind, _, _)| kind != "open")
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let revision = rig.c.graph.snapshot().active_revision.unwrap();
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(kind, r, _)| kind == "open" && *r == Some(revision))
    );
    rig.request(AppRequest::Stop);
    assert_eq!(
        adapter.revision.load(Ordering::Acquire),
        0,
        "Stop invalidates pending gate jobs immediately"
    );
    rig.wait(|r| {
        r.driver.cleaning == 0 && !r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
    });
    assert_eq!(
        rig.c.exports.statuses(&rig.c.graph.snapshot())[0].resource,
        ExportResourceState::Present
    );
    let calls = adapter.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .all(|(kind, _, thread)| kind != "remove" && thread == "app-prepare")
    );
}

#[test]
fn export_local_eligibility_recreates_after_drain_and_requires_fresh_apply() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let id = rig.c.graph.snapshot().exports[0].endpoint_id;
    rig.request(AppRequest::SetExportAuthorizations(vec![
        ExportAuthorization {
            endpoint_id: id,
            allow_default: true,
        },
    ]));
    rig.wait(|r| {
        let statuses = r.c.exports.statuses(&r.c.graph.snapshot());
        statuses[0].resource == ExportResourceState::Present
            && statuses[0].allow_default
            && r.c.exports.observations[&id].allow_default
    });
    assert_eq!(rig.c.graph.snapshot().active_revision, None);
    assert!(!rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(kind, _, _)| kind == "remove")
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
}

#[test]
fn unrelated_local_export_authorization_does_not_stop_running_owner() {
    let (mut rig, _) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    rig.request(AppRequest::SetExportAuthorizations(vec![
        ExportAuthorization {
            endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x72; 16]),
            allow_default: true,
        },
    ]));
    for _ in 0..15 {
        rig.tick();
    }
    assert_eq!(rig.state.stops.load(Ordering::Acquire), 0);
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    assert!(rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
}

#[test]
fn failed_export_retirement_stays_removing_until_driver_reports_absence() {
    let (mut rig, adapter) = export_rig();
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Present
    });
    adapter.fail_remove.store(true, Ordering::Release);
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(Vec::new())));
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Removing
    });
    assert!(!rig.driver.exports_runtime.is_drained());
    adapter.fail_remove.store(false, Ordering::Release);
    rig.wait(|r| r.driver.exports_runtime.is_drained());
}

#[test]
fn explicit_exit_requests_retirement_without_claiming_client_held_pdo_absence() {
    let (mut rig, adapter) = export_rig();
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Present
    });
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    rig.wait(|r| r.driver.is_drained() && r.pool.is_idle());
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(kind, _, _)| kind == "remove")
    );
    assert!(
        rig.c
            .exports
            .observations
            .values()
            .all(|o| o.resource == ExportResourceState::Removing && !o.gate_open)
    );
}

#[test]
fn explicit_exit_with_unknown_create_finishes_without_fabricating_absence() {
    let (mut rig, adapter) = export_rig();
    adapter.uncertain_create.store(true, Ordering::Release);
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Creating
    });
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    rig.wait(|r| r.driver.is_drained() && r.pool.is_idle());
    assert!(rig.c.exports.observations.values().all(|o| {
        o.resource == ExportResourceState::Creating && !o.gate_open && o.active_revision.is_none()
    }));
}

#[test]
fn explicit_exit_does_not_wait_for_failed_native_retirement_to_report_absence() {
    let (mut rig, adapter) = export_rig();
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Present
    });
    adapter.fail_remove.store(true, Ordering::Release);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    rig.wait(|r| r.driver.is_drained() && r.pool.is_idle());
    assert_eq!(rig.state.stops.load(Ordering::Acquire), 1);
    assert!(
        rig.c
            .graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("shutdown REMOVE timeout"))
    );
    assert!(rig.c.exports.observations.values().all(|o| {
        o.resource == ExportResourceState::Removing && !o.gate_open && o.active_revision.is_none()
    }));
}

#[test]
fn export_name_apply_drains_owner_and_recreates_from_intent() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let mut intent = rig.c.graph.snapshot().exports[0].clone();
    intent.name = "Renamed playback from project".into();
    let id = intent.endpoint_id;
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![
        intent.clone(),
    ])));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.exports
            .observations
            .get(&id)
            .is_some_and(|o| o.intent.name == intent.name && o.gate_open)
    });
    assert_eq!(rig.state.starts.load(Ordering::Acquire), 2);
    assert_eq!(rig.state.stops.load(Ordering::Acquire), 1);
    assert_eq!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _, _)| kind == "create")
            .count(),
        2
    );
}

#[test]
fn unapplied_export_unbinding_and_removal_keep_confirmed_resources() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let active = rig.c.graph.snapshot().active_revision;
    let mut intent = rig.c.graph.snapshot().exports[0].clone();
    intent.binding = None;
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![intent])));
    for _ in 0..15 {
        rig.tick();
    }
    assert!(rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![])));
    for _ in 0..15 {
        rig.tick();
    }
    let statuses = rig.c.exports.statuses(&rig.c.graph.snapshot());
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].active_revision, active);
    assert!(statuses[0].gate_open);
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(kind, _, _)| kind != "remove")
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.observations.is_empty());
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(kind, _, _)| kind == "remove")
    );
}

#[test]
fn stop_and_exit_revoke_a_pending_gate_dispatch_without_blocking_pump() {
    for exiting in [false, true] {
        let (mut rig, adapter) = export_rig();
        adapter.hold_open.store(true, Ordering::Release);
        rig.request(AppRequest::ApplyGraph);
        rig.wait(|_| adapter.open_entered.load(Ordering::Acquire));
        let started = Instant::now();
        if exiting {
            rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
        } else {
            rig.request(AppRequest::Stop);
        }
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(adapter.revision.load(Ordering::Acquire), 0);
        adapter.hold_open.store(false, Ordering::Release);
        if exiting {
            rig.wait(|r| r.driver.is_drained() && r.pool.is_idle());
        } else {
            rig.wait(|r| r.driver.cleaning == 0 && r.pool.is_idle());
            assert_eq!(
                rig.c.exports.statuses(&rig.c.graph.snapshot())[0].resource,
                ExportResourceState::Present
            );
            assert!(!rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
        }
        assert!(
            adapter
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(kind, _, _)| kind != "open")
        );
        assert_eq!(rig.c.graph.snapshot().last_error, None);
    }
}

#[test]
fn engine_recovery_closes_export_gate_until_owner_and_revision_are_confirmed() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    rig.state.fail_owner.store(true, Ordering::SeqCst);
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Failed
            && !r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
    });
    assert_eq!(adapter.revision.load(Ordering::Acquire), 0);
    assert_eq!(
        rig.c.exports.statuses(&rig.c.graph.snapshot())[0].active_revision,
        None
    );
    let host_id = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert_eq!(
        rig.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
    let output = rig.node(NodeKind::Sink);
    let project = rig.driver.owner.as_ref().unwrap().observer.project();
    rig.state.fail_owner.store(false, Ordering::SeqCst);
    rig.state
        .events
        .lock()
        .unwrap()
        .push_back(host::ResourceEvent {
            node: output,
            configuration: project.nodes[&output].clone(),
            generation: 1,
            available: true,
        });
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
    });
    assert_eq!(
        rig.c.exports.statuses(&rig.c.graph.snapshot())[0].active_revision,
        rig.c.graph.snapshot().active_revision
    );
    assert_eq!(
        rig.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
}

#[test]
fn initial_export_apply_prepares_pcm_before_waiting_for_transport_admission() {
    let (mut rig, adapter) = export_rig();
    adapter.transport_unavailable.store(true, Ordering::Release);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    let host_id = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    assert_eq!(
        rig.c.exports.statuses(&rig.c.graph.snapshot())[0].resource,
        ExportResourceState::Present
    );
    assert!(!rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
    adapter
        .transport_unavailable
        .store(false, Ordering::Release);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    assert_eq!(
        rig.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn unbound_export_without_pcm_does_not_block_later_graph_applies() {
    let (mut rig, adapter) = export_rig();
    let mut intent = rig.c.graph.snapshot().exports[0].clone();
    intent.binding = None;
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![intent])));
    adapter.transport_unavailable.store(true, Ordering::Release);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    let host_id = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert!(
        !rig.driver.apply,
        "an unbound export has no PCM worker to become transport-ready"
    );
    rig.wait(|r| {
        r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(
        rig.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    let status = &rig.c.exports.statuses(&rig.c.graph.snapshot())[0];
    assert_eq!(status.resource, ExportResourceState::Present);
    assert!(!status.gate_open);
    assert_eq!(status.active_revision, None);
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(kind, _, _)| kind != "open")
    );
}

#[test]
fn apply_replaces_waiting_owner_after_export_pcm_drains_with_present_pdo() {
    let (mut rig, adapter) = export_rig();
    struct ReleasePreparation(Arc<State>);
    impl Drop for ReleasePreparation {
        fn drop(&mut self) {
            self.0.block_update.store(false, Ordering::SeqCst);
        }
    }
    // Release the fake preparation before Rig's shutdown even if an assertion
    // fails, so this regression cannot strand the test process's cleanup join.
    let _release = ReleasePreparation(rig.state.clone());
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let host_id = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    let intent = rig.c.graph.snapshot().exports[0].clone();
    rig.state.block_update.store(true, Ordering::SeqCst);
    rig.state.ignore_update_cancel.store(true, Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.state.prepares.load(Ordering::SeqCst) == 2);
    adapter.transport_unavailable.store(true, Ordering::Release);
    rig.state.fail_owner.store(true, Ordering::SeqCst);
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Failed
            && !r
                .driver
                .exports_runtime
                .ready_for(&r.c.exports, std::slice::from_ref(&intent))
    });
    assert!(
        rig.driver
            .exports_runtime
            .lifecycle_ready_for(&rig.c.exports, std::slice::from_ref(&intent)),
        "a drained PCM worker does not remove or fault the PDO"
    );
    assert_eq!(
        rig.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .read()
            .state
            .execution,
        ExecutionState::WaitingForResources
    );
    rig.state.fail_owner.store(false, Ordering::SeqCst);
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert!(
        rig.driver.owner.is_none(),
        "Apply must retire the stranded host before preparing a replacement PCM worker"
    );
    assert!(rig.driver.apply);
    assert_eq!(adapter.revision.load(Ordering::Acquire), 0);
    adapter
        .transport_unavailable
        .store(false, Ordering::Release);
    for _ in 0..3 {
        rig.tick();
        assert!(rig.driver.owner.is_none());
        assert_eq!(rig.driver.cleaning, 1);
        assert_eq!(rig.state.prepares.load(Ordering::SeqCst), 2);
        assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    }
    rig.state.block_update.store(false, Ordering::SeqCst);
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
    });
    assert_ne!(
        rig.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(rig.state.stops.load(Ordering::SeqCst), 1);
    assert!(
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(kind, _, _)| kind != "remove"),
        "PCM recovery must retain the healthy PDO"
    );
}

#[test]
fn nondefault_f32_and_f64_settings_support_export_admission_after_confirmation() {
    for (precision, asio) in [
        (ProcessingPrecision::F32, false),
        (ProcessingPrecision::F64, false),
        (ProcessingPrecision::F32, true),
        (ProcessingPrecision::F64, true),
    ] {
        let (mut rig, _) = export_rig();
        let backend = FakeBackend(rig.state.clone());
        rig.driver.f64_factory = Some(Arc::new(move |_, _| Ok(Box::new(backend.clone()))));
        let mut settings = AudioSettings::default();
        settings.processing.precision = precision;
        settings.processing.sample_rate = 44_100;
        settings.processing.block_frames = 512;
        if asio {
            settings.backend = AudioBackend::Asio;
        }
        rig.request(AppRequest::SetAudioSettings(settings.clone()));
        rig.request(AppRequest::ApplyGraph);
        rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
        assert_eq!(rig.c.active_audio_settings, Some(settings));
        assert_eq!(
            rig.c.exports.statuses(&rig.c.graph.snapshot())[0].active_revision,
            rig.c.graph.snapshot().active_revision
        );
        rig.request(AppRequest::Stop);
        rig.wait(|r| {
            r.driver.cleaning == 0 && !r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
        });
        assert_eq!(
            rig.c.exports.statuses(&rig.c.graph.snapshot())[0].resource,
            ExportResourceState::Present
        );
    }
}

#[test]
fn unsupported_export_apply_preserves_the_confirmed_owner_and_revision() {
    use super::super::{ExportDirection, ExportIntent};
    use moiren_engine::node::VirtualEndpointId;
    let mut rig = Rig::new();
    rig.start();
    let active = rig.c.graph.snapshot().active_revision;
    let generation = rig.c.core.generation();
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![
        ExportIntent {
            endpoint_id: VirtualEndpointId::from_bytes([0x81; 16]),
            direction: ExportDirection::Playback,
            name: "Playback".into(),
            channels: 2,
            enabled: true,
            binding: Some(source),
        },
    ])));
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.sample_rate = 96_000;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert_eq!(rig.c.graph.snapshot().active_revision, active);
    assert_eq!(rig.c.core.generation(), generation);
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert_eq!(rig.state.stops.load(Ordering::SeqCst), 0);
    assert_eq!(rig.c.active_audio_settings, Some(AudioSettings::default()));
    assert!(
        rig.c
            .graph
            .snapshot()
            .last_error
            .unwrap()
            .contains("not integrated")
    );
}

#[test]
fn invalid_export_candidate_keeps_the_confirmed_admission_before_commit() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let active = rig.c.graph.snapshot().active_revision;
    let generation = rig.c.core.generation();
    let revision = adapter.revision.load(Ordering::Acquire);
    let gate_calls = || {
        adapter
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _, _)| kind == "open" || kind == "close")
            .count()
    };
    let before = gate_calls();
    rig.request(AppRequest::EditGraph(GraphEdit::ClearBinding(
        rig.node(NodeKind::Sink),
    )));
    rig.request(AppRequest::ApplyGraph);
    assert_eq!(
        adapter.revision.load(Ordering::Acquire),
        revision,
        "request receipt cannot revoke a confirmed graph before candidate validation"
    );
    rig.tick();
    assert!(rig.c.graph.snapshot().last_error.is_some());
    assert!(!rig.driver.apply);
    for _ in 0..15 {
        rig.tick();
    }
    assert_eq!(rig.c.graph.snapshot().active_revision, active);
    assert_eq!(rig.c.core.generation(), generation);
    assert_eq!(rig.state.stops.load(Ordering::Acquire), 0);
    assert!(rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
    assert_eq!(adapter.revision.load(Ordering::Acquire), revision);
    assert_eq!(
        gate_calls(),
        before,
        "rejected candidate must not cycle a live gate"
    );
}

#[test]
fn invalid_start_candidate_keeps_the_confirmed_export_admission() {
    let (mut rig, adapter) = export_rig();
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let active = rig.c.graph.snapshot().active_revision;
    let revision = adapter.revision.load(Ordering::Acquire);
    // Start configures only the primary IO pair. This additional unbound sink
    // is legal editable topology but cannot be prepared as an Engine project.
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Sink,
        channels: 2,
    }));
    rig.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    assert_eq!(adapter.revision.load(Ordering::Acquire), revision);
    rig.tick();
    assert!(rig.c.graph.snapshot().last_error.is_some());
    assert!(!rig.driver.apply);
    assert_eq!(rig.c.graph.snapshot().active_revision, active);
    assert_eq!(rig.state.stops.load(Ordering::Acquire), 0);
    assert_eq!(adapter.revision.load(Ordering::Acquire), revision);
    assert!(rig.c.exports.statuses(&rig.c.graph.snapshot())[0].gate_open);
}

#[test]
fn asio_automatic_startup_stays_idle_until_a_driver_is_explicitly_bound() {
    let mut rig = Rig::new();
    rig.c.audio_settings.backend = AudioBackend::Asio;
    rig.driver.automatic_startup = true;
    let catalog = DeviceCatalog {
        inputs: vec![DeviceRow {
            endpoint_id: "wasapi-input".into(),
            name: "Microphone".into(),
        }],
        outputs: vec![DeviceRow {
            endpoint_id: "wasapi-output".into(),
            name: "Speakers".into(),
        }],
        default_input_endpoint_id: Some("wasapi-input".into()),
        default_output_endpoint_id: Some("wasapi-output".into()),
        asio_drivers: vec![
            DeviceRow {
                endpoint_id: "asio:{first}".into(),
                name: "First".into(),
            },
            DeviceRow {
                endpoint_id: "asio:{second}".into(),
                name: "Second".into(),
            },
        ],
        ..Default::default()
    };
    rig.driver
        .queue()
        .port()
        .try_reserve()
        .unwrap()
        .send_or_dispose(Completion::Catalog(Ok(catalog)), drop);
    rig.tick();
    assert!(rig.driver.owner.is_none());
    assert!(!rig.driver.automatic_startup);
    assert_eq!(rig.c.core.phase(), SessionPhase::Idle);
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), 0);
    assert!(
        rig.c
            .graph
            .snapshot()
            .nodes
            .iter()
            .all(|node| node.output_endpoint_id.is_none())
    );
    rig.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: rig.node(NodeKind::Source),
        source: InputSelection::Physical {
            endpoint_id: "asio:{second}".into(),
        },
    }));
    rig.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: rig.node(NodeKind::Sink),
        endpoint_id: "asio:{second}".into(),
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| rig.c.core.phase() == SessionPhase::Running);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.c.core.desired().unwrap().output_endpoint_id,
        "asio:{second}"
    );
}

#[test]
fn audio_settings_are_draft_until_apply_and_restart_reaps_previous_owner() {
    let mut rig = Rig::new();
    rig.start();
    assert_eq!(rig.c.active_audio_settings, Some(AudioSettings::default()));
    let first_generation = rig.c.core.generation();
    let mut settings = AudioSettings::default();
    settings.processing.sample_rate = 96_000;
    settings.processing.block_frames = 512;
    rig.request(AppRequest::SetAudioSettings(settings.clone()));
    rig.tick();
    assert_eq!(rig.c.audio_settings, settings);
    assert_eq!(rig.c.active_audio_settings, Some(AudioSettings::default()));
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert_eq!(rig.c.active_audio_settings, None);
    rig.wait(|r| r.c.active_audio_settings.as_ref() == Some(&settings));
    assert_eq!(rig.c.active_latency_profile, None);
    assert_ne!(rig.c.core.generation(), first_generation);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(rig.state.stops.load(Ordering::SeqCst), 1);
    rig.request(AppRequest::Stop);
    assert_eq!(rig.c.active_audio_settings, None);
    rig.wait(|r| r.driver.is_drained());
}

#[test]
fn settings_draft_after_apply_does_not_change_the_configuration_being_applied() {
    let mut rig = Rig::new();
    rig.start();
    let mut applied = AudioSettings::default();
    applied.processing.sample_rate = 96_000;
    let mut draft = applied.clone();
    draft.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(applied.clone()));
    rig.request(AppRequest::ApplyGraph);
    rig.request(AppRequest::SetAudioSettings(draft.clone()));
    rig.wait(|r| r.c.active_audio_settings.as_ref() == Some(&applied));
    assert_eq!(rig.c.audio_settings, draft);
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(rig.c.active_audio_settings, Some(applied));
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_audio_settings_preserve_requested_and_running_configuration() {
    let mut rig = Rig::new();
    rig.start();
    let mut settings = AudioSettings::default();
    settings.input.buffer_frames = Some(96);
    rig.request(AppRequest::SetAudioSettings(settings));
    assert_eq!(rig.c.audio_settings, AudioSettings::default());
    assert_eq!(rig.c.active_audio_settings, Some(AudioSettings::default()));
    assert!(rig.c.graph.snapshot().last_error.is_some());
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
}

#[test]
fn start_session_block_size_matches_requested_and_confirmed_processing() {
    let mut rig = Rig::new();
    rig.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 512,
    }));
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    assert_eq!(rig.c.audio_settings.processing.block_frames, 512);
    assert_eq!(
        rig.c
            .active_audio_settings
            .as_ref()
            .unwrap()
            .processing
            .block_frames,
        512
    );
    assert_eq!(rig.c.core.desired().unwrap().max_block_frames, 512);
    assert_eq!(
        rig.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .read()
            .desired
            .processing
            .engine
            .max_block_frames,
        512
    );
}

#[test]
fn f32_only_factory_reports_unsupported_precision_without_opening_a_host() {
    let mut rig = Rig::new();
    rig.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: rig.node(NodeKind::Sink),
        endpoint_id: "output".into(),
    }));
    let mut settings = AudioSettings::default();
    settings.processing.precision = ProcessingPrecision::F64;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    assert_eq!(rig.c.active_audio_settings, None);
    assert!(rig.c.graph.snapshot().last_error.unwrap().contains("f64"));
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn f64_settings_open_the_f64_host_and_retire_on_precision_change() {
    let mut rig = Rig::new();
    rig.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: rig.node(NodeKind::Sink),
        endpoint_id: "output".into(),
    }));
    let backend = FakeBackend(rig.state.clone());
    rig.driver = ManagedDriver::new(|_| {
        Err(host::BackendError::Unsupported(
            "f32 should not be selected".into(),
        ))
    })
    .with_f64_factory(move |_| Ok(Box::new(backend.clone())));
    let mut settings = AudioSettings::default();
    settings.processing.precision = ProcessingPrecision::F64;
    settings.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(settings.clone()));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    let owner = rig.driver.owner.as_ref().unwrap();
    assert!(matches!(owner.host, AudioHost::F64(_)));
    assert_eq!(
        owner
            .observer
            .read()
            .desired
            .processing
            .engine
            .processing_sr,
        44_100.0
    );
    assert_eq!(rig.c.active_audio_settings, Some(settings));
    rig.request(AppRequest::SetAudioSettings(AudioSettings::default()));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    assert_eq!(rig.state.stops.load(Ordering::SeqCst), 1);
    assert_eq!(rig.c.active_audio_settings, None);
}

#[test]
fn each_strip_publishes_its_own_post_fader_level_only_on_demand() {
    let mut rig = Rig::new();
    rig.start();
    let gain = rig.node(NodeKind::Gain);
    let pan = rig.node(NodeKind::Pan);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !rig.c.signals.topics.node_levels.contains_key(&gain) {
        assert!(Instant::now() < deadline);
        rig.tick();
    }
    let topic = rig.c.signals.topics.node_levels[&gain].clone();
    assert_eq!(topic.business_subscribers(), 0);
    assert_eq!(
        rig.c.signals.topics.node_levels[&pan].business_subscribers(),
        0
    );
    let (mut ticket, request) =
        crate::service::signals::subscription_request(topic.clone(), Default::default());
    rig.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: gain,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    loop {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        if let Some(window) = subscriber.read_latest()
            && window.header().frame_end > window.header().frame_start
            && window.elements().iter().all(|level| {
                (level.peak - 0.5).abs() < 0.00001 && (level.rms - 0.5).abs() < 0.00001
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "gain strip must publish its actual post-fader samples: {:?}",
            rig.c.graph.snapshot().last_error
        );
    }
    assert_eq!(
        rig.c.signals.topics.node_levels[&pan].business_subscribers(),
        0
    );
    drop(subscriber);
    assert_eq!(topic.business_subscribers(), 0);
}

#[test]
fn compressor_updates_are_acknowledged_live_without_rebuilding_or_touching_other_nodes() {
    use crate::service::CompressorParameter;
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Compressor,
        channels: 2,
    }));
    let compressor = rig.node(NodeKind::Compressor);
    let snapshot = rig.c.graph.snapshot();
    let source = rig.node(NodeKind::Source);
    let sink = rig.node(NodeKind::Sink);
    let final_edge = snapshot
        .graph
        .edges()
        .iter()
        .find(|edge| edge.dst() == sink)
        .unwrap()
        .id();
    rig.request(AppRequest::EditGraph(GraphEdit::Disconnect(final_edge)));
    for (source, destination) in [(source, compressor), (compressor, sink)] {
        rig.request(AppRequest::EditGraph(GraphEdit::Connect {
            source: snapshot.graph.get_node(source).unwrap().outputs()[0].id(),
            destination: snapshot.graph.get_node(destination).unwrap().inputs()[0].id(),
        }));
    }
    rig.request(AppRequest::EditGraph(GraphEdit::SetCompressor {
        node: compressor,
        parameter: CompressorParameter::Ratio,
        value: 1.,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 1.
    });
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::SetCompressor {
        node: compressor,
        parameter: CompressorParameter::OutputGain,
        value: -6.020599913279624,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && (rig.sample() - 0.5).abs() < 0.00001
    });
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.c.graph.snapshot().node(source).unwrap().mixer,
        snapshot.node(source).unwrap().mixer
    );
}

#[test]
fn mixer_edits_wait_for_apply_then_activate_without_changing_sends() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    let before = rig.c.graph.snapshot();
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!(rig.sample(), 1.);
    assert_eq!(
        rig.c.graph.snapshot().active_revision,
        before.active_revision
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 0.5
    });
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(rig.c.graph.snapshot().graph.edges(), before.graph.edges());
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            muted: true,
            ..Default::default()
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 0.
    });
}
impl Rig {
    fn new() -> Self {
        let backend = FakeBackend::default();
        let state = backend.0.clone();
        let driver = ManagedDriver::new(move |_| Ok(Box::new(backend.clone())));
        Self {
            driver,
            c: ServiceContext::default(),
            pool: JobPool::new().unwrap(),
            state,
        }
    }
    fn request(&mut self, request: AppRequest) {
        self.driver.request(&mut self.c, &mut self.pool, request);
    }
    fn tick(&mut self) {
        self.tick_with_budget(RoundBudget::default());
    }
    fn tick_with_budget(&mut self, budget: RoundBudget) {
        self.driver.poll(&mut self.c, &mut self.pool, budget);
        self.c.dispatch(&mut self.pool);
        std::thread::sleep(Duration::from_millis(2));
    }
    fn wait(&mut self, predicate: impl Fn(&Self) -> bool) {
        self.wait_with_budget(predicate, RoundBudget::default());
    }
    fn wait_with_budget(&mut self, predicate: impl Fn(&Self) -> bool, budget: RoundBudget) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(self) {
            assert!(
                Instant::now() < deadline,
                "phase {:?}, graph {:?}",
                self.c.core.phase(),
                self.c.graph.snapshot()
            );
            self.tick_with_budget(budget);
        }
    }
    fn start(&mut self) {
        self.request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        }));
        self.wait(|r| r.c.core.phase() == SessionPhase::Running);
    }
    fn confirmation_revision(&self) -> Option<(u64, u64)> {
        self.driver
            .confirm_parameters
            .map(|confirmation| (confirmation.revision, confirmation.host_revision))
    }
    fn sample(&self) -> f64 {
        f64::from_bits(self.state.last_sample.load(Ordering::SeqCst))
    }
    fn node(&self, kind: NodeKind) -> moiren_core::graph::NodeId {
        self.c
            .graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == kind)
            .unwrap()
            .id()
    }
}

#[test]
fn late_topology_activation_preserves_newer_parameter_apply_ack_barrier() {
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let topology_revision = r.c.graph.snapshot().candidate_revision;
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    let no_parameter_submissions = RoundBudget {
        applied: 0,
        ..RoundBudget::default()
    };
    r.wait_with_budget(
        |r| r.confirmation_revision() == Some((parameter_revision, 2)),
        no_parameter_submissions,
    );
    assert!(!r.driver.confirmation_values.is_empty());
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait_with_budget(
        |r| {
            r.driver.edits.is_empty()
                && r.driver
                    .owner
                    .as_ref()
                    .unwrap()
                    .observer
                    .snapshot()
                    .active_revision
                    == Some(2)
        },
        no_parameter_submissions,
    );
    assert_eq!(r.confirmation_revision(), Some((parameter_revision, 2)));
    assert!(!r.driver.confirmation_values.is_empty());
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    assert_eq!(
        r.c.graph.snapshot().pending_revision,
        Some(parameter_revision)
    );
    assert_ne!(
        r.driver.owner.as_ref().unwrap().active_revision.1,
        topology_revision
    );
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    assert_eq!(r.sample(), 0.25);
}

#[test]
fn late_topology_activation_cannot_regress_completed_newer_parameter_apply() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.driver.owner.as_ref().unwrap().active_revision == (2, parameter_revision));
    assert_eq!(
        r.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .snapshot()
            .active_revision,
        Some(1)
    );
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(
        r.driver.owner.as_ref().unwrap().active_revision,
        (2, parameter_revision)
    );
    assert_eq!(
        r.c.graph.snapshot().active_revision,
        Some(parameter_revision)
    );
}

#[test]
fn queued_same_structure_apply_requires_acks_despite_an_older_observer_basis() {
    use std::sync::atomic::AtomicBool;
    struct ReleaseOnDrop(Arc<AtomicBool>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let release_on_drop = ReleaseOnDrop(release.clone());
    let callback_entered = entered.clone();
    let callback_release = release.clone();
    let _gate_receipt = r
        .driver
        .owner
        .as_ref()
        .unwrap()
        .control
        .edit(move |_| {
            callback_entered.store(true, Ordering::SeqCst);
            while !callback_release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
        .unwrap();
    r.wait(|_| entered.load(Ordering::SeqCst));
    r.state.pause_audio.store(true, Ordering::SeqCst);
    let no_parameter_submissions = RoundBudget {
        applied: 0,
        ..RoundBudget::default()
    };
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick_with_budget(no_parameter_submissions);
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick_with_budget(no_parameter_submissions);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    assert_eq!(
        r.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .snapshot()
            .desired_revision,
        1
    );
    drop(release_on_drop);
    r.wait_with_budget(
        |r| r.confirmation_revision() == Some((parameter_revision, 2)),
        no_parameter_submissions,
    );
    assert!(!r.driver.confirmation_values.is_empty());
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    assert_eq!(r.sample(), 0.25);
}

#[test]
fn apply_ack_for_a_new_parameter_target_waits_for_pending_topology_mapping() {
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    let gain = r.c.graph.snapshot().graph.nodes().last().unwrap().id();
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.confirmation_revision() == Some((revision, 2)));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    assert!(r.c.graph.snapshot().last_error.is_none());
    assert!(
        r.driver
            .parameters
            .iter()
            .any(|pending| pending
                .confirmation
                .is_some_and(|confirmation| confirmation.revision == revision
                    && confirmation.host_revision == 2))
    );
    assert!(
        r.driver
            .parameters
            .iter()
            .all(|pending| !matches!(pending.receipt.outcome(), ParameterOutcome::Rejected(_)))
    );
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn repeated_same_candidate_apply_keeps_its_own_future_target_ack_barrier() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| {
        r.confirmation_revision() == Some((revision, 2)) && r.driver.confirmation_values.is_empty()
    });
    for _ in 0..10 {
        r.tick();
    }
    let older = r
        .driver
        .parameters
        .iter()
        .find(|pending| {
            pending.confirmation.is_some_and(|confirmation| {
                confirmation.revision == revision && confirmation.host_revision == 2
            }) && pending.receipt.outcome() == ParameterOutcome::Accepted
        })
        .expect("new Gain request waits for topology mapping")
        .receipt
        .clone();
    r.request(AppRequest::ApplyGraph);
    r.wait(|_| older.outcome() == ParameterOutcome::Superseded);
    for _ in 0..3 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().candidate_revision, revision);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    assert_eq!(r.confirmation_revision(), Some((revision, 2)));
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| {
        r.c.graph.snapshot().active_revision == Some(revision) && r.c.control.accepted_pending == 0
    });
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.c.core.begin_exit();
        self.driver.begin_exit(&mut self.c, &mut self.pool);
        self.wait(|r| r.driver.is_drained() && r.pool.is_idle() && r.c.pending_jobs() == 0);
    }
}

#[test]
fn compatible_topology_swap_retains_owner_epoch_and_confirms_latest_revision() {
    let mut r = Rig::new();
    r.start();
    let before = r.driver.owner.as_ref().unwrap().observer.snapshot();
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    let revision = r.c.graph.snapshot().candidate_revision;
    assert_ne!(r.c.graph.snapshot().active_revision, Some(revision));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    let after = r.driver.owner.as_ref().unwrap().observer.snapshot();
    assert_eq!(before.owner_generation, after.owner_generation);
    assert_eq!(
        before.timeline.unwrap().epoch,
        after.timeline.unwrap().epoch
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.sample(), 1.0);
}

#[test]
fn rejected_compilation_preserves_running_output_and_active_revision() {
    let mut r = Rig::new();
    r.start();
    let active = r.c.graph.snapshot().active_revision;
    let receipt = r
        .driver
        .owner
        .as_ref()
        .unwrap()
        .control
        .edit(|p| {
            p.processing.audio_byte_budget = 1;
            Ok(())
        })
        .unwrap();
    r.wait(|r| {
        matches!(
            r.driver.owner.as_ref().unwrap().observer.snapshot().update,
            UpdateState::Blocked { .. }
        )
    });
    assert!(matches!(receipt.outcome(), EditOutcome::Accepted { .. }));
    assert_eq!(r.c.core.phase(), SessionPhase::Running);
    assert_eq!(r.c.graph.snapshot().active_revision, active);
    assert_eq!(r.sample(), 1.0);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn node_send_and_stereo_parameters_receive_actual_realtime_acks() {
    let mut r = Rig::new();
    r.start();
    let gain = r.node(NodeKind::Gain);
    let pan = r.node(NodeKind::Pan);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.5,
    }));
    r.request(AppRequest::EditGraph(GraphEdit::SetPan {
        node: pan,
        pan: 0.5,
    }));
    r.request(AppRequest::EditGraph(GraphEdit::SetMono {
        node: pan,
        mono: true,
    }));
    r.wait(|r| {
        (r.sample() - 0.375).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
    let edge = r.c.graph.snapshot().graph.edges()[2].id();
    r.request(AppRequest::EditGraph(GraphEdit::SetSend {
        edge,
        params: moiren_core::graph::SendParams {
            gain: 0.5,
            ..Default::default()
        },
    }));
    r.wait(|r| {
        (r.sample() - 0.1875).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
}

#[test]
fn loaded_document_equal_node_ids_cannot_control_previous_project() {
    let mut r = Rig::new();
    r.start();
    let old_host = r.driver.owner.as_ref().unwrap().observer.read().host_id;
    let snapshot = r.c.graph.snapshot();
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::ReplaceGraph {
        graph: snapshot.graph,
        nodes: snapshot.nodes,
    });
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(r.sample(), 1.0);
    assert_eq!(r.c.control.accepted_pending, 0);
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && (r.sample() - 0.25).abs() < 1e-6);
    assert_ne!(
        old_host,
        r.driver.owner.as_ref().unwrap().observer.read().host_id
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn stop_cancels_pending_preparation_and_destroys_owners_off_service_thread() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::Stop);
    assert_eq!(r.c.active_latency_profile, None);
    r.wait(|r| r.driver.is_drained() && r.c.core.phase() == SessionPhase::Idle);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    assert!(
        r.state
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|name| name != "fake-audio-owner" && name != "app-service")
    );
}

#[test]
fn legacy_meter_subscription_receives_managed_host_audio_windows() {
    let mut r = Rig::new();
    let (mut ticket, request) =
        signals::subscription_request(r.c.signals.topics.tone_output.clone(), Default::default());
    r.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    r.start();
    r.wait(|r| {
        r.driver
            .meters
            .as_ref()
            .is_some_and(|_| r.state.last_sample.load(Ordering::SeqCst) != 0)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        r.tick();
        r.c.signals.pump_if_due(Instant::now());
        if let Some(window) = subscriber.read_latest() {
            assert_eq!(window.elements()[0].peak, 1.0);
            assert_eq!(window.elements()[0].rms, 1.0);
            assert!(window.header().frame_end > window.header().frame_start);
            break;
        }
        assert!(Instant::now() < deadline, "managed meter did not publish");
    }
}

#[test]
fn multiple_inputs_mix_through_bus_and_feed_every_bound_output() {
    let mut r = Rig::new();
    for kind in [NodeKind::Source, NodeKind::Sink, NodeKind::Bus] {
        r.request(AppRequest::EditGraph(GraphEdit::AddNode {
            kind,
            channels: 2,
        }));
    }
    let snapshot = r.c.graph.snapshot();
    let source = snapshot.graph.nodes()[4].id();
    let output = snapshot.graph.nodes()[5].id();
    let bus = snapshot.graph.nodes()[6].id();
    let first_output = r.node(NodeKind::Sink);
    let pan = r.node(NodeKind::Pan);
    r.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: source,
        source: InputSelection::Tone {
            frequency_hz: 880.0,
        },
    }));
    r.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: output,
        endpoint_id: "second".into(),
    }));
    r.request(AppRequest::EditGraph(GraphEdit::AddBusInput(bus)));
    r.request(AppRequest::EditGraph(GraphEdit::AddBusInput(bus)));
    let snapshot = r.c.graph.snapshot();
    r.request(AppRequest::EditGraph(GraphEdit::Disconnect(
        snapshot.graph.edges()[2].id(),
    )));
    let ports = [
        (
            snapshot.graph.get_node(pan).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(bus).unwrap().inputs()[0].id(),
        ),
        (
            snapshot.graph.get_node(source).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(bus).unwrap().inputs()[1].id(),
        ),
        (
            snapshot.graph.get_node(bus).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(first_output).unwrap().inputs()[0].id(),
        ),
        (
            snapshot.graph.get_node(bus).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(output).unwrap().inputs()[0].id(),
        ),
    ];
    for (source, destination) in ports {
        r.request(AppRequest::EditGraph(GraphEdit::Connect {
            source,
            destination,
        }));
    }
    r.start();
    r.wait(|r| r.sample() == 2.0);
    assert_eq!(r.state.sources.load(Ordering::SeqCst), 2);
    assert_eq!(r.state.sinks.load(Ordering::SeqCst), 2);
}

#[test]
fn profile_selection_waits_until_apply_and_replaces_the_host() {
    let mut r = Rig::new();
    r.start();
    r.request(AppRequest::SetLatencyProfile(AudioLatencyProfile::Low));
    for _ in 0..3 {
        r.tick();
    }
    assert_eq!(r.c.latency_profile, AudioLatencyProfile::Low);
    assert_eq!(
        r.c.active_latency_profile,
        Some(AudioLatencyProfile::Compatible)
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.active_latency_profile == Some(AudioLatencyProfile::Low));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn unavailable_resources_report_diagnostics_and_stop_cancels_host() {
    let mut r = Rig::new();
    r.state.unavailable.store(true, Ordering::SeqCst);
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "missing".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.graph.snapshot().last_error.is_some());
    assert_ne!(r.c.core.phase(), SessionPhase::Running);
    assert_eq!(r.c.graph.snapshot().active_revision, None);
    assert_eq!(r.c.active_latency_profile, None);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn newest_apply_supersedes_pending_candidate_without_replacing_owner() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick();
    let revision = r.c.graph.snapshot().candidate_revision;
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.sample(), 1.0);
}

#[test]
fn initial_resource_recovery_restores_confirmed_revision_and_phase() {
    let mut r = Rig::new();
    r.state.unavailable.store(true, Ordering::SeqCst);
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    let host_id = r.driver.owner.as_ref().unwrap().observer.read().host_id;
    r.request(AppRequest::ApplyGraph);
    r.tick();
    assert_eq!(
        r.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
    let output = r.node(NodeKind::Sink);
    let project = r.driver.owner.as_ref().unwrap().observer.project();
    r.state.unavailable.store(false, Ordering::SeqCst);
    r.state
        .events
        .lock()
        .unwrap()
        .push_back(host::ResourceEvent {
            node: output,
            configuration: project.nodes[&output].clone(),
            generation: 1,
            available: true,
        });
    r.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(r.c.core.error(), None);
    assert_eq!(
        r.c.active_latency_profile,
        Some(AudioLatencyProfile::Compatible)
    );
    assert_eq!(
        r.driver.owner.as_ref().unwrap().observer.read().host_id,
        host_id
    );
}

#[test]
#[ignore = "wall-clock delivery benchmark; run explicitly on an otherwise idle host"]
fn meter_bridge_delivers_fresh_windows_within_one_120_hz_frame() {
    run_meter_delivery_benchmark(96, true);
}

#[test]
#[ignore = "wall-clock callback-batching benchmark; run explicitly on an otherwise idle host"]
fn meter_bridge_reports_large_callback_batching_at_120_hz() {
    run_meter_delivery_benchmark(960, false);
}

fn run_meter_delivery_benchmark(callback_frames: usize, require_frame_age: bool) {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    // Match wall-clock callback cadence to the 48 kHz processing clock.
    state
        .timed_meter_frames
        .store(callback_frames, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.5,
            pan: 0.0,
            // The processing capacity must use a supported block size; the
            // backend still renders the requested (possibly smaller) callback.
            max_block_frames: callback_frames.max(256).next_power_of_two(),
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let topic = loop {
        if let Some(snapshot) = handle.try_snapshot()
            && snapshot.phase == SessionPhase::Running
        {
            break snapshot.signal_topics.tone_output.clone();
        }
        assert!(Instant::now() < deadline, "service did not start");
        std::thread::sleep(Duration::from_millis(1));
    };
    let mut ticket = handle.subscribe_topic(topic, Default::default()).unwrap();
    let mut subscriber = loop {
        if let Some(subscriber) = ticket.try_take().unwrap() {
            break subscriber;
        }
        assert!(Instant::now() < deadline, "subscription did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    };
    let mut ages = Vec::new();
    let mut sequences = Vec::new();
    let mut observed_windows = 0;
    let mut latest_frame_end = None;
    let mut missing_updates = 0;
    let period = Duration::from_secs_f64(1.0 / 120.0);
    let mut next = Instant::now() + period;
    // Warm up subscriptions, then consume at a native-frame-equivalent cadence.
    for frame in 0..150 {
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        next += period;
        let mut received = false;
        if let Some(window) = subscriber.read_latest() {
            received = true;
            observed_windows += 1;
            assert_eq!(window.header().frame_end - window.header().frame_start, 200);
            latest_frame_end = Some(window.header().frame_end);
            if frame >= 30 {
                sequences.push(window.meta().sequence);
            }
        }
        if frame >= 30 {
            missing_updates += usize::from(!received);
            let read_at = Instant::now();
            let history = state.render_history.lock().unwrap();
            if let Some(frame_end) = latest_frame_end
                && let Some((_, published_at)) = history.iter().find(|(end, _)| *end >= frame_end)
            {
                // Include cached-window age when this tick receives nothing.
                // Selecting only fresh deliveries would hide stale UI frames.
                ages.push(read_at.saturating_duration_since(*published_at));
            }
        }
    }
    handle.request_exit();
    runtime.join().unwrap();
    eprintln!(
        "observed windows={observed_windows}, post-warmup ticks without update={missing_updates}, callback count={}, final state={:?}",
        state.render_history.lock().unwrap().len(),
        handle.try_snapshot().map(|s| (s.phase, s.error.clone()))
    );
    assert_eq!(
        ages.len(),
        120,
        "every post-warmup consumption tick must measure its newest available window"
    );
    let mut callback_gaps = state
        .render_history
        .lock()
        .unwrap()
        .iter()
        .map(|(_, time)| *time)
        .collect::<Vec<_>>()
        .windows(2)
        .map(|pair| pair[1].duration_since(pair[0]))
        .collect::<Vec<_>>();
    ages.sort_unstable();
    callback_gaps.sort_unstable();
    let p95 = ages[(ages.len() * 95).div_ceil(100) - 1];
    let callback_p95 = callback_gaps[(callback_gaps.len() * 95).div_ceil(100) - 1];
    eprintln!(
        "meter delivery: samples={}, age p95={p95:?}, callback p95={callback_p95:?}, callback max={:?}, first/last sequence={:?}/{:?}",
        ages.len(),
        callback_gaps.last(),
        sequences.first(),
        sequences.last()
    );
    assert!(
        sequences.windows(2).all(|pair| pair[1] > pair[0]),
        "new deliveries replayed an old sequence"
    );
    assert!(
        sequences.len() >= if require_frame_age { 100 } else { 30 },
        "too few fresh deliveries across 120 ticks: {}, missing updates: {missing_updates}",
        sequences.len()
    );
    if require_frame_age {
        assert!(
            p95 <= period,
            "meter delivery age {p95:?} exceeds a 120 Hz frame ({period:?})"
        );
    } else {
        assert!(
            missing_updates > 0,
            "large callbacks must expose batching rather than invented measurements"
        );
        assert!(
            p95 < Duration::from_millis(250),
            "callback-batched windows became stale"
        );
    }
}

#[test]
fn service_exit_joins_managed_host_and_preserves_retained_observer_diagnostics() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.0,
            pan: 0.0,
            max_block_frames: 256,
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle
        .try_snapshot()
        .is_none_or(|s| s.phase != SessionPhase::Running)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    handle.request_exit();
    while handle
        .try_snapshot()
        .is_none_or(|s| s.phase != SessionPhase::Exited)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    runtime.join().unwrap();
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    assert!(
        state
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|name| name != "app-service")
    );
}

#[test]
fn parameter_only_apply_waits_for_its_own_ack_when_older_requests_fill_the_ledger() {
    let mut r = Rig::new();
    for _ in 0..64 {
        r.request(AppRequest::EditGraph(GraphEdit::AddNode {
            kind: NodeKind::Gain,
            channels: 2,
        }));
    }
    r.start();
    let old = r.c.graph.snapshot().active_revision;
    r.state.pause_audio.store(true, Ordering::SeqCst);
    let nodes: Vec<_> =
        r.c.graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Gain)
            .take(64)
            .map(|n| n.id())
            .collect();
    for node in nodes {
        r.request(AppRequest::EditGraph(GraphEdit::SetGain {
            node,
            gain: 0.5,
        }));
    }
    r.wait(|r| r.driver.parameters.len() == 64);
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.driver.confirm_parameters.is_some());
    for _ in 0..5 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().active_revision, old);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    // NextBlock commands may be consumed late under coordinator contention.
    // Both replies prove application; this test checks ACK ownership, not a deadline.
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
}

#[test]
fn unchanged_apply_stays_pending_until_its_own_realtime_ack() {
    let mut rig = Rig::new();
    rig.start();
    let revision = rig.c.graph.snapshot().candidate_revision;
    rig.state.pause_audio.store(true, Ordering::SeqCst);
    // Let any already-entered callback finish before submitting new requests.
    for _ in 0..3 {
        rig.tick();
    }
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert_eq!(rig.c.graph.snapshot().active_revision, Some(revision));
    assert_eq!(rig.c.graph.snapshot().pending_revision, Some(revision));
    rig.wait(|rig| rig.driver.confirm_parameters.is_some());
    for _ in 0..5 {
        rig.tick();
        assert_eq!(rig.c.graph.snapshot().pending_revision, Some(revision));
    }
    rig.state.pause_audio.store(false, Ordering::SeqCst);
    rig.wait(|rig| rig.c.graph.snapshot().pending_revision.is_none());
    assert_eq!(rig.c.graph.snapshot().active_revision, Some(revision));
    assert!(rig.driver.confirm_parameters.is_none());
}

#[test]
fn later_live_edit_supersedes_parameter_confirmation_without_replaying_old_value() {
    let mut r = Rig::new();
    r.start();
    let gain = r.node(NodeKind::Gain);
    let old = r.c.graph.snapshot().active_revision;
    r.state.pause_audio.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.driver.confirm_parameters.is_some());
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.75,
    }));
    assert_eq!(r.driver.confirm_parameters, None);
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| {
        (r.sample() - 0.75).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
    assert_eq!(r.c.graph.snapshot().active_revision, old);
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert_eq!(r.sample(), 0.75);
}

#[test]
fn backend_factory_panic_is_a_failed_snapshot_and_service_can_exit() {
    let mut driver = ManagedDriver::new(|_| panic!("injected backend factory panic"));
    let mut c = ServiceContext::default();
    let mut pool = JobPool::new().unwrap();
    driver.request(
        &mut c,
        &mut pool,
        AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.0,
            pan: 0.0,
            max_block_frames: 256,
        }),
    );
    driver.poll(&mut c, &mut pool, RoundBudget::default());
    assert_eq!(c.core.phase(), SessionPhase::Failed);
    assert!(c.core.error().unwrap().contains("factory panicked"));
    assert!(driver.is_drained());
}

#[test]
fn replacement_document_fences_a_late_previous_project_preparation() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let snapshot = r.c.graph.snapshot();
    r.request(AppRequest::ReplaceGraph {
        graph: snapshot.graph,
        nodes: snapshot.nodes,
    });
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.75,
    }));
    r.state.block_update.store(false, Ordering::SeqCst);
    for _ in 0..20 {
        r.tick();
    }
    assert_eq!(r.sample(), 1.0);
    assert_ne!(
        r.c.graph.snapshot().active_revision,
        Some(r.c.graph.snapshot().candidate_revision)
    );
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && (r.sample() - 0.75).abs() < 1e-6);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
}
