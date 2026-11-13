use super::*;
use crate::service::test_backend::{FakeBackend, State};
use moiren_core::graph::NodeKind;
use std::sync::atomic::{AtomicBool, Ordering};

struct Rig {
    driver: ManagedDriver,
    c: ServiceContext,
    pool: JobPool,
    state: Arc<State>,
}

/// Poll the recording worker's written-frame counter until it passes
/// `beyond`, returning the observed value. Realtime-render independent, so
/// loaded CI runners cannot starve the assertion.
fn wait_written_frames(rig: &mut Rig, recorder: NodeId, beyond: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let frames = rig
            .driver
            .recording_sessions
            .get(&recorder)
            .map(|recording| recording.session.status().written_frames)
            .unwrap_or(0);
        if frames > beyond {
            return frames;
        }
        assert!(
            Instant::now() < deadline,
            "written_frames stuck at {frames} (beyond {beyond})"
        );
        rig.tick();
    }
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
}

#[test]
fn virtual_to_physical_apply_keeps_the_session_intent_across_owner_swaps() {
    let (mut rig, _) = export_rig();
    rig.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    assert!(rig.c.core.desired().is_none());
    let generation = rig.c.core.generation();
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(Vec::new())));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running && r.c.core.generation() != generation);
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
fn export_presence_does_not_open_gate_until_engine_confirms_and_exit_revokes_it() {
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
    rig.c.core.begin_exit();
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    assert_eq!(
        adapter.revision.load(Ordering::Acquire),
        0,
        "Exit invalidates pending gate jobs immediately"
    );
    rig.wait(|r| {
        r.driver.is_drained() && !r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
    });
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
fn exit_revokes_a_pending_gate_dispatch_without_blocking_pump() {
    let (mut rig, adapter) = export_rig();
    adapter.hold_open.store(true, Ordering::Release);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|_| adapter.open_entered.load(Ordering::Acquire));
    let started = Instant::now();
    rig.c.core.begin_exit();
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(adapter.revision.load(Ordering::Acquire), 0);
    adapter.hold_open.store(false, Ordering::Release);
    rig.wait(|r| r.driver.is_drained() && r.pool.is_idle());
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
        rig.c.core.begin_exit();
        rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
        rig.wait(|r| {
            r.driver.is_drained() && !r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open
        });
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
    assert_eq!(
        rig.c.active_latency_profile,
        Some(crate::service::AudioLatencyProfile::Compatible)
    );
    assert_ne!(rig.c.core.generation(), first_generation);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(rig.state.stops.load(Ordering::SeqCst), 1);
    rig.c.core.begin_exit();
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
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
    let source = rig.c.signals.topics.node_levels[&gain].clone();
    assert_eq!(source.business_subscribers(), 0);
    assert_eq!(
        rig.c.signals.topics.node_levels[&pan].business_subscribers(),
        0
    );
    let lease = source.subscribe();
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
        let mut matched = false;
        lease.read_latest(|_epoch, window| {
            matched |= window.header().frame_end > window.header().frame_start
                && window.elements().iter().all(|level| {
                    (level.peak - 0.5).abs() < 0.00001 && (level.rms - 0.5).abs() < 0.00001
                });
        });
        if matched {
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
    drop(lease);
    assert_eq!(source.business_subscribers(), 0);
}

/// A running rig with a spectrum analyzer spliced between the pan and the
/// sink, ticked until the bridge registered the analyzer's topic.
fn spectrum_rig() -> (Rig, moiren_core::graph::NodeId) {
    let mut rig = Rig::new();
    // Pace the fake owner at the session's 256-frame block maximum (5.3 ms
    // callbacks): the 8192-point default fills in 32 renders instead of a
    // thousand millisecond-tick ones, which a coarse Windows timer could
    // stretch past the publication deadline.
    rig.state.timed_meter_frames.store(256, Ordering::Relaxed);
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::SpectrumAnalyzer,
        channels: 2,
    }));
    let spectrum = rig.node(NodeKind::SpectrumAnalyzer);
    let snapshot = rig.c.graph.snapshot();
    let pan = rig.node(NodeKind::Pan);
    let sink = rig.node(NodeKind::Sink);
    let final_edge = snapshot
        .graph
        .edges()
        .iter()
        .find(|edge| edge.dst() == sink)
        .unwrap()
        .id();
    rig.request(AppRequest::EditGraph(GraphEdit::Disconnect(final_edge)));
    for (source, destination) in [(pan, spectrum), (spectrum, sink)] {
        rig.request(AppRequest::EditGraph(GraphEdit::Connect {
            source: snapshot.graph.get_node(source).unwrap().outputs()[0].id(),
            destination: snapshot.graph.get_node(destination).unwrap().inputs()[0].id(),
        }));
    }
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| rig.c.signals.topics.node_spectra.contains_key(&spectrum));
    (rig, spectrum)
}

/// Pump the rig until a delivered window satisfies `accept`, which also
/// asserts whatever must hold for every window on the way.
fn wait_spectrum(
    rig: &mut Rig,
    subscriber: &mut moiren_signal::Subscriber<signals::SpectrumTopic>,
    mut accept: impl FnMut(&moiren_engine::spectrum::SpectrumWindow, &[f32]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        let mut matched = false;
        while let Some(window) = subscriber.next() {
            matched |= accept(window.header(), window.elements());
        }
        if matched {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "spectrum analyzer did not publish: {:?}",
            rig.c.graph.snapshot().last_error
        );
    }
}

#[test]
fn spectrum_bridge_relays_fft_windows_only_on_demand() {
    let (mut rig, spectrum) = spectrum_rig();
    let topic = rig.c.signals.topics.node_spectra[&spectrum].clone();
    assert_eq!(topic.business_subscribers(), 0);
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(
        rig.driver.spectra.as_ref().unwrap().demanded_spectra(),
        0,
        "no subscriber, no engine-side FFT demand"
    );
    let (mut ticket, request) =
        crate::service::signals::subscription_request(topic.clone(), Default::default());
    rig.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    // The fake backend feeds DC 1.0: once the analysis window has filled,
    // the one-sided magnitudes read 1.0 at the DC bin of each channel and
    // nothing mid-band, channel-major at the default 8192-point shape.
    wait_spectrum(&mut rig, &mut subscriber, |header, bins| {
        assert_eq!(header.fft_size, 8192);
        assert_eq!(header.channels, 2);
        assert!(header.sample_rate > 0.);
        assert!(header.frame_end > header.frame_start);
        assert_eq!(bins.len(), 2 * 4097);
        (bins[0] - 1.).abs() < 0.02 && (bins[4097] - 1.).abs() < 0.02 && bins[2048].abs() < 1e-3
    });
    assert_eq!(rig.driver.spectra.as_ref().unwrap().demanded_spectra(), 1);
    drop(subscriber);
    assert_eq!(topic.business_subscribers(), 0);
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(
        rig.driver.spectra.as_ref().unwrap().demanded_spectra(),
        0,
        "dropping the last subscriber releases the engine subscription"
    );
}

#[test]
fn fft_resize_reregisters_the_spectrum_topic() {
    let (mut rig, spectrum) = spectrum_rig();
    let before = rig.c.signals.topics.node_spectra[&spectrum].clone();
    rig.request(AppRequest::EditGraph(GraphEdit::SetBuiltin {
        node: spectrum,
        state: crate::service::BuiltinState::SpectrumAnalyzer(
            moiren_engine::spectrum::SpectrumSettings {
                fft_size: 4096,
                hop_frames: 2048,
            },
        ),
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c
            .signals
            .topics
            .node_spectra
            .get(&spectrum)
            .is_some_and(|topic| *topic != before)
    });
    let after = rig.c.signals.topics.node_spectra[&spectrum].clone();
    let (mut ticket, request) =
        crate::service::signals::subscription_request(after, Default::default());
    rig.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    wait_spectrum(&mut rig, &mut subscriber, |header, bins| {
        assert_eq!(header.fft_size, 4096);
        assert_eq!(bins.len(), 2 * 2049);
        true
    });
    assert_eq!(rig.c.signals.topics.node_spectra.len(), 1);
}

/// A departing node's tap must leave the registry (and the snapshot's
/// node map) without a trace and without an error, so the UI's resolve
/// drops the subscription instead of chasing a dead tap.
#[test]
fn removing_a_node_closes_its_level_tap_without_stalling_the_registry() {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    let bus = rig.node(NodeKind::Bus);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| rig.c.signals.topics.node_levels.contains_key(&bus));
    rig.request(AppRequest::EditGraph(GraphEdit::RemoveNode(bus)));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        !rig.c.signals.topics.node_levels.contains_key(&bus)
            && rig.c.graph.snapshot().active_revision
                == Some(rig.c.graph.snapshot().candidate_revision)
    });
    for _ in 0..5 {
        rig.tick();
    }
    let snapshot = rig.c.graph.snapshot();
    assert!(
        !snapshot
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("Node meters")),
        "{:?}",
        snapshot.last_error
    );
    assert_eq!(
        rig.c.signals.topics.node_levels.len(),
        snapshot.graph.nodes().len()
    );
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
fn mixer_edits_apply_live_without_rebuilding_or_waiting_for_apply() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    let before = rig.c.graph.snapshot();
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    rig.wait(|rig| rig.sample() == 0.5);
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.c.graph.snapshot().active_revision,
        before.active_revision
    );
    assert_eq!(rig.c.graph.snapshot().graph.edges(), before.graph.edges());
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            muted: true,
            ..Default::default()
        },
    }));
    rig.wait(|rig| rig.sample() == 0.);
}

#[test]
fn topology_edits_auto_apply_after_the_debounce_without_a_request() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    // No ApplyGraph request is issued: the rebuild fires on its own.
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert!(rig.state.prepares.load(Ordering::SeqCst) > prepares);
}

#[test]
fn failed_auto_apply_keeps_audio_running_and_the_next_edit_retries() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    rig.wait(|rig| rig.sample() == 0.5);
    rig.state.fail_update.store(true, Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    // The automatic rebuild fails; audio keeps running on the old graph.
    rig.wait(|rig| rig.c.graph.snapshot().last_error.is_some());
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    assert_eq!(rig.sample(), 0.5);
    // The next edit retries the rebuild and this time it activates.
    rig.state.fail_update.store(false, Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.c.graph.snapshot().last_error.is_none()
    });
    assert_eq!(rig.sample(), 0.5);
}

/// Design 23 §6 (audit fact F6, the ghost rewrite): with no selection ledger
/// left to mirror, clearing the primary Source's binding and starting by
/// graph leaves the node unbound, fails on the node it names, and repairs
/// cleanly once the binding is restored.
#[test]
fn clearing_the_primary_source_binding_fails_on_the_node_and_repairs_cleanly() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::ClearBinding(source)));
    rig.request(AppRequest::ApplyGraph);
    // The always-on session keeps running: the rejection lands on the node
    // it names instead of stopping audio.
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Running && !r.c.graph.snapshot().faults.is_empty()
    });
    let snapshot = rig.c.graph.snapshot();
    assert!(
        snapshot
            .nodes
            .iter()
            .find(|entry| entry.id == source)
            .unwrap()
            .source
            .is_none(),
        "nothing rewrites a stale selection back into the node"
    );
    assert!(snapshot.faults.iter().any(|fault| {
        fault.subject == GraphSubject::Node(source)
            && fault.cause == "The Source node is not bound to an input source"
    }));
    // The repair clears the fault group on the next successful edit, and
    // the following apply runs the repaired graph.
    rig.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: source,
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
    }));
    rig.wait(|r| r.c.graph.snapshot().faults.is_empty());
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
}

#[test]
fn solo_edits_fan_out_live_strip_mutes_to_parallel_sources() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Source,
        channels: 2,
    }));
    let other = rig
        .c
        .graph
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Source && node.id() != source)
        .unwrap()
        .id();
    rig.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: other,
        source: crate::service::InputSelection::Tone {
            frequency_hz: 440.0,
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let other = rig
        .c
        .graph
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Source && node.id() != source)
        .unwrap()
        .id();
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    // Soloing the chain source silences the parallel branch live.
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            solo: true,
            ..Default::default()
        },
    }));
    rig.wait(|rig| {
        rig.driver.strip_live.get(&other) == Some(&(1.0, true))
            && rig.driver.unsent.is_empty()
            && rig.driver.parameters.is_empty()
    });
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    // The soloed chain itself stays audible at unity gain.
    assert_eq!(rig.driver.strip_live.get(&source), Some(&(1.0, false)));
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
        // Generous slack: the rig polls a 2 ms cadence, so progress normally
        // lands in milliseconds; the margin only matters when a loaded CI
        // runner starves the rig's threads for seconds at a time.
        let deadline = Instant::now() + Duration::from_secs(20);
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
    // The repeated apply re-establishes its own barrier once its host edit
    // activates; the supersede may be observed first.
    r.wait(|r| {
        r.c.graph.snapshot().candidate_revision == revision
            && r.c.graph.snapshot().pending_revision == Some(revision)
            && r.confirmation_revision() == Some((revision, 2))
    });
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
fn exit_cancels_pending_preparation_and_destroys_owners_off_service_thread() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.c.core.begin_exit();
    r.driver.begin_exit(&mut r.c, &mut r.pool);
    assert_eq!(r.c.active_latency_profile, None);
    r.wait(|r| r.driver.is_drained() && r.c.core.phase() == SessionPhase::Exiting);
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
    // Demand exists before the session: the lease counts, and the
    // registry binds the engine subscription on the first poll with an
    // owner.
    let lease = r.c.signals.topics.tone_output.subscribe();
    r.start();
    r.wait(|r| {
        r.driver
            .spectra
            .as_ref()
            .is_some_and(|_| r.state.last_sample.load(Ordering::SeqCst) != 0)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        r.tick();
        let mut delivered = false;
        lease.read_latest(|_epoch, window| {
            assert_eq!(window.elements()[0].peak, 1.0);
            assert_eq!(window.elements()[0].rms, 1.0);
            assert!(window.header().frame_end > window.header().frame_start);
            delivered = true;
        });
        if delivered {
            break;
        }
        assert!(Instant::now() < deadline, "managed meter did not publish");
    }
}

/// Decision D-4 route B regression (the deleted relay gate's semantic on
/// the direct path): a UI lease reads the engine's windows with no relay
/// copy in between, keeps receiving them across a graph hot swap, releases
/// the engine demand completely when dropped, and a re-shown lease picks
/// fresh windows immediately while a sibling lease kept the topic warm.
#[test]
fn direct_level_lease_delivers_windows_across_hot_swap_and_hide_show() {
    let mut rig = Rig::new();
    rig.start();
    let sink = rig.node(NodeKind::Sink);
    rig.wait(|rig| rig.c.signals.topics.node_levels.contains_key(&sink));
    let source = rig.c.signals.topics.node_levels[&sink].clone();
    let mut lease = source.subscribe();
    rig.wait(|rig| rig.c.levels.demanded_levels() == 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let last_frame_end = loop {
        rig.tick();
        if let Some(header) = lease.read_latest(|_epoch, window| *window.header()) {
            assert!(header.timeline_epoch != 0);
            assert!(header.frame_end > header.frame_start);
            break header.frame_end;
        }
        assert!(
            Instant::now() < deadline,
            "direct lease received no windows"
        );
    };
    // A sibling lease stands in for an independent analysis reader.
    let analysis = source.subscribe();
    assert_eq!(source.business_subscribers(), 2);
    // Graph hot swap: the plan rebuilds in place, and the window stream
    // must not stall (this continuity is why the relay existed).
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        rig.tick();
        if lease
            .read_latest(|_epoch, window| window.header().frame_end > last_frame_end)
            .unwrap_or(false)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "windows stalled across the hot swap: {:?}",
            rig.c.graph.snapshot().last_error
        );
    }
    // Hide: dropping the lease releases its engine subscription outright.
    drop(lease);
    rig.wait(|rig| rig.c.levels.demanded_levels() == 1);
    assert_eq!(source.business_subscribers(), 1);
    // Show again: the fresh lease must reach windows without a generation
    // reset, and the frames keep advancing past everything seen before.
    lease = source.subscribe();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        rig.tick();
        if lease
            .read_latest(|_epoch, window| window.header().frame_end > last_frame_end)
            .unwrap_or(false)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "re-shown lease received no windows"
        );
    }
    drop(lease);
    drop(analysis);
    rig.wait(|rig| rig.c.levels.demanded_levels() == 0);
    assert_eq!(source.business_subscribers(), 0);
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
fn unavailable_resources_report_diagnostics_and_exit_drains_the_host() {
    let mut r = Rig::new();
    r.state.unavailable.store(true, Ordering::SeqCst);
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "missing".into(),
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.graph.snapshot().last_error.is_some());
    assert_ne!(r.c.core.phase(), SessionPhase::Running);
    assert_eq!(r.c.graph.snapshot().active_revision, None);
    assert_eq!(r.c.active_latency_profile, None);
    r.c.core.begin_exit();
    r.driver.begin_exit(&mut r.c, &mut r.pool);
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
            gain: 0.5,
            pan: 0.0,
            // The processing capacity must use a supported block size; the
            // backend still renders the requested (possibly smaller) callback.
            max_block_frames: callback_frames.max(256).next_power_of_two(),
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let source = loop {
        if let Some(snapshot) = handle.try_snapshot()
            && snapshot.phase == SessionPhase::Running
        {
            break snapshot.signal_topics.tone_output.clone();
        }
        assert!(Instant::now() < deadline, "service did not start");
        std::thread::sleep(Duration::from_millis(1));
    };
    // The registry binds the engine subscription on its first poll; the
    // lease reads the newest window directly (one copy, no relay).
    let lease = source.subscribe();
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
        let received = lease
            .read_latest(|_epoch, window| {
                observed_windows += 1;
                assert_eq!(window.header().frame_end - window.header().frame_start, 200);
                (
                    window.header().frame_end,
                    frame >= 30,
                    window.meta().sequence,
                )
            })
            .map(|(frame_end, counted, sequence)| {
                latest_frame_end = Some(frame_end);
                if counted {
                    sequences.push(sequence);
                }
            })
            .is_some();
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

#[test]
fn undo_restored_candidates_rebuild_after_the_debounce() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let nodes_before = rig.c.graph.snapshot().nodes.len();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    rig.request(AppRequest::UndoGraph);
    // The restored candidate activates through the same trailing debounce
    // as structural edits — no new start, no explicit apply request.
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(rig.c.graph.snapshot().nodes.len(), nodes_before);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert!(rig.state.prepares.load(Ordering::SeqCst) > prepares);
    assert!(
        rig.c.graph.snapshot().can_redo,
        "the undone step awaits redo"
    );
    rig.request(AppRequest::RedoGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.c.graph.snapshot().nodes.len() == nodes_before + 1
    });
}

#[test]
fn auto_apply_reopens_a_failed_session_once_the_fault_is_repaired() {
    let mut rig = Rig::new();
    // No owner and an unbound default graph: the automatic apply fails on
    // the nodes instead of opening a host.
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        rig.tick();
    }
    rig.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    assert!(rig.driver.owner.is_none());
    // Repairing the bindings is itself a structural edit: the always-on
    // auto-apply re-opens the session without an explicit request.
    rig.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: rig.node(NodeKind::Source),
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
    }));
    rig.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: rig.node(NodeKind::Sink),
        endpoint_id: "output".into(),
    }));
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    assert!(rig.driver.owner.is_some());
}

#[test]
fn undo_on_a_running_session_restores_the_candidate_and_reapplies_it() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    let nodes_before = rig.c.graph.snapshot().nodes.len();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
    });
    rig.request(AppRequest::UndoGraph);
    // Drain the debounce window: the candidate restores and auto-applies,
    // keeping the always-on session on the restored graph.
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        rig.tick();
    }
    rig.wait(|rig| {
        rig.c.graph.snapshot().nodes.len() == nodes_before
            && rig.c.graph.snapshot().active_revision
                == Some(rig.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
}

/// Catalog fake driving `with_catalog`: rows or a sticky failure, never a session.
struct FakeCatalog {
    failure: AtomicBool,
    empty: AtomicBool,
    calls: AtomicUsize,
}

impl FakeCatalog {
    fn filled() -> Self {
        Self {
            failure: AtomicBool::new(false),
            empty: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        }
    }
}

impl crate::service::CatalogBackend for FakeCatalog {
    fn catalog(&self) -> Result<crate::service::DeviceCatalog, crate::service::BackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failure.load(Ordering::SeqCst) {
            return Err(crate::service::BackendError::new(
                "catalog",
                "fake enumeration failure",
            ));
        }
        Ok(if self.empty.load(Ordering::SeqCst) {
            crate::service::DeviceCatalog::default()
        } else {
            crate::service::DeviceCatalog {
                inputs: vec![crate::service::DeviceRow {
                    endpoint_id: "input".into(),
                    name: "input".into(),
                }],
                outputs: vec![crate::service::DeviceRow {
                    endpoint_id: "output".into(),
                    name: "output".into(),
                }],
                default_input_endpoint_id: Some("input".into()),
                default_output_endpoint_id: Some("output".into()),
                ..Default::default()
            }
        })
    }
}

fn catalog_rig(catalog: Arc<FakeCatalog>, automatic_startup: bool) -> Rig {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let driver = ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))).with_catalog(catalog);
    let driver = if automatic_startup {
        driver.with_automatic_startup()
    } else {
        driver
    };
    Rig {
        driver,
        c: ServiceContext::default(),
        pool: JobPool::new().unwrap(),
        state,
    }
}

#[test]
fn automatic_startup_uses_catalog_defaults_once_and_catalog_refreshes_do_not_restart() {
    let mut rig = catalog_rig(Arc::new(FakeCatalog::filled()), true);
    rig.tick();
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    let desired = rig.c.core.desired().expect("default session").clone();
    assert_eq!(desired.gain, 0.0);
    assert_eq!(
        desired.source,
        crate::service::InputSelection::Physical {
            endpoint_id: "input".into()
        }
    );
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    let generation = rig.c.core.generation();
    rig.request(AppRequest::RefreshCatalog);
    rig.wait(|r| r.c.catalog_error.is_none() && r.c.catalog.outputs.len() == 1);
    assert_eq!(rig.c.core.generation(), generation);
    assert!(rig.c.core.desired().is_some());
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
}

/// A service whose context already holds a restored project, the way
/// `start_service` seeds it before the driver polls.
fn resume_rig(catalog: Arc<FakeCatalog>, document: crate::project::ProjectDocument) -> Rig {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let driver = ManagedDriver::new(move |_| Ok(Box::new(backend.clone())))
        .with_catalog(catalog)
        .with_automatic_resume();
    let mut c = ServiceContext::default();
    c.load_project(document).expect("restored project loads");
    Rig {
        driver,
        c,
        pool: JobPool::new().unwrap(),
        state,
    }
}

/// The default chain bound to the fake catalog's devices at a non-default
/// gain, plus whatever `extend` adds, saved as a project document.
fn restored_document(
    extend: impl FnOnce(&mut crate::service::ServiceGraph),
) -> crate::project::ProjectDocument {
    let mut graph = crate::service::ServiceGraph::default();
    let snapshot = graph.snapshot();
    let source = snapshot.node_of_kind(NodeKind::Source).unwrap().0.id();
    let sink = snapshot.node_of_kind(NodeKind::Sink).unwrap().0.id();
    let gain = snapshot.node_of_kind(NodeKind::Gain).unwrap().0.id();
    graph
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
        })
        .unwrap();
    graph
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "output".into(),
        })
        .unwrap();
    graph
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.7,
        })
        .unwrap();
    extend(&mut graph);
    crate::project::ProjectDocument::from_graph("Restored", &graph.snapshot()).unwrap()
}

/// Design 23 §4.3: documents that predate the `processing` field carry the
/// block size on the session, because the deleted UI ledger used to ferry it
/// into a Start request. `load_project` migrates it so start-by-graph opens
/// with the size the project was saved with; explicit processing stays
/// authoritative when both forms are present.
#[test]
fn loading_a_legacy_document_migrates_the_session_block_size() {
    let legacy_session = SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 512,
    };
    let mut document = restored_document(|_| {});
    document.processing = None;
    document.session = Some((&legacy_session).into());
    let mut context = ServiceContext::default();
    context
        .load_project(document)
        .expect("legacy document loads");
    assert_eq!(context.audio_settings.processing.block_frames, 512);

    let mut document = restored_document(|_| {});
    document.session = Some((&legacy_session).into());
    document.processing = Some(moiren_engine::audio_config::ProcessingSettings {
        block_frames: 128,
        ..Default::default()
    });
    let mut context = ServiceContext::default();
    context
        .load_project(document)
        .expect("mixed document loads");
    assert_eq!(
        context.audio_settings.processing.block_frames, 128,
        "explicit processing wins over the session block size"
    );
}

fn add_physical_source(graph: &mut crate::service::ServiceGraph, label: &str, endpoint_id: &str) {
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Source,
            channels: 2,
        })
        .unwrap();
    let node = graph
        .snapshot()
        .graph
        .nodes()
        .iter()
        .rfind(|node| node.kind() == NodeKind::Source)
        .unwrap()
        .id();
    graph
        .edit(GraphEdit::SetLabel {
            node,
            label: label.into(),
        })
        .unwrap();
    graph
        .edit(GraphEdit::BindSource {
            node,
            source: InputSelection::Physical {
                endpoint_id: endpoint_id.into(),
            },
        })
        .unwrap();
}

fn settled(rig: &Rig) -> bool {
    let graph = rig.c.graph.snapshot();
    graph.pending_revision.is_none() && graph.active_revision == Some(graph.candidate_revision)
}

#[test]
fn restored_project_resumes_with_its_own_bindings_once_the_catalog_arrives() {
    let mut rig = resume_rig(Arc::new(FakeCatalog::filled()), restored_document(|_| {}));
    let before = rig.c.graph.snapshot();
    rig.tick();
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running && settled(r));
    let snapshot = rig.c.graph.snapshot();
    assert_eq!(
        snapshot.candidate_revision, before.candidate_revision,
        "every device is present: resume edits nothing"
    );
    assert_eq!(snapshot.graph.nodes().len(), before.graph.nodes().len());
    // Automatic startup would have rebound the catalog defaults and muted
    // the chain; resume keeps the restored bindings and gain.
    assert_eq!(
        snapshot.node_of_kind(NodeKind::Source).unwrap().1.source,
        Some(InputSelection::Physical {
            endpoint_id: "input".into()
        })
    );
    assert_eq!(snapshot.node_of_kind(NodeKind::Gain).unwrap().1.gain, 0.7);
    assert!(snapshot.faults.is_empty());
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn resume_strips_offline_secondary_sources_and_reports_once_running() {
    let document = restored_document(|graph| {
        add_physical_source(graph, "Unplugged mic", "missing-input");
    });
    let mut rig = resume_rig(Arc::new(FakeCatalog::filled()), document);
    let before = rig.c.graph.snapshot().candidate_revision;
    rig.tick();
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running && settled(r));
    let snapshot = rig.c.graph.snapshot();
    assert!(
        snapshot
            .nodes
            .iter()
            .all(|node| node.label != "Unplugged mic"),
        "the offline source leaves so the rest of the graph can run"
    );
    assert_eq!(
        snapshot
            .graph
            .nodes()
            .iter()
            .filter(|node| node.kind() == NodeKind::Source)
            .count(),
        1
    );
    assert_eq!(
        snapshot.candidate_revision,
        before + 1,
        "one strip, one edit"
    );
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    // The notice lands only after activation, which clears the fault group.
    rig.wait(|r| !r.c.graph.snapshot().faults.is_empty());
    let faults = rig.c.graph.snapshot().faults;
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].subject, GraphSubject::Graph);
    assert!(
        faults[0].cause.contains("'Unplugged mic' (input)"),
        "{}",
        faults[0].cause
    );
    assert!(
        faults[0]
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("undo"))
    );
    rig.tick();
    rig.tick();
    assert_eq!(
        rig.c.graph.snapshot().faults.len(),
        1,
        "periodic confirmation must not clear the published notice"
    );
    // The strip is an ordinary journaled edit: undo brings the node back.
    rig.request(AppRequest::UndoGraph);
    assert!(
        rig.c
            .graph
            .snapshot()
            .nodes
            .iter()
            .any(|node| node.label == "Unplugged mic")
    );
}

#[test]
fn resume_never_strips_the_last_source_and_leaves_its_device_to_the_host() {
    let document = restored_document(|graph| {
        let source = graph
            .snapshot()
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        graph
            .edit(GraphEdit::BindSource {
                node: source,
                source: InputSelection::Physical {
                    endpoint_id: "missing-input".into(),
                },
            })
            .unwrap();
    });
    let mut rig = resume_rig(Arc::new(FakeCatalog::filled()), document);
    let before = rig.c.graph.snapshot();
    rig.tick();
    // The apply still goes out: a real host parks in WaitingForResources
    // and recovers when the device returns, which stripping would forfeit.
    rig.wait(|r| r.state.starts.load(Ordering::SeqCst) == 1);
    let snapshot = rig.c.graph.snapshot();
    assert_eq!(snapshot.candidate_revision, before.candidate_revision);
    assert_eq!(snapshot.graph.nodes().len(), before.graph.nodes().len());
    assert_eq!(
        snapshot.node_of_kind(NodeKind::Source).unwrap().1.source,
        Some(InputSelection::Physical {
            endpoint_id: "missing-input".into()
        })
    );
    assert!(rig.driver.resume_notice.is_none());
}

#[test]
fn explicit_apply_before_the_catalog_cancels_automatic_resume() {
    let mut rig = resume_rig(Arc::new(FakeCatalog::filled()), restored_document(|_| {}));
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    rig.wait(|r| {
        r.c.catalog.outputs.len() == 1
            && !r.driver.catalog.is_running()
            && r.c.core.phase() == SessionPhase::Running
    });
    assert!(!rig.driver.automatic_resume);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn catalog_failure_is_visible_and_refresh_preserves_running_session_and_last_rows() {
    let catalog = Arc::new(FakeCatalog::filled());
    catalog.failure.store(true, Ordering::SeqCst);
    let mut rig = catalog_rig(catalog.clone(), false);
    rig.tick();
    rig.wait(|r| {
        r.c.catalog_error
            .as_deref()
            .is_some_and(|error| error.contains("fake enumeration failure"))
    });
    assert_eq!(rig.c.core.phase(), SessionPhase::Idle);
    assert!(rig.c.catalog.outputs.is_empty());
    assert_eq!(rig.c.core.error(), None);
    catalog.failure.store(false, Ordering::SeqCst);
    rig.request(AppRequest::RefreshCatalog);
    rig.wait(|r| r.c.catalog_error.is_none() && !r.c.catalog.outputs.is_empty());
    let rows = rig.c.catalog.clone();
    rig.request(AppRequest::Start(crate::service::SessionSpec {
        source: crate::service::InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    rig.wait(|r| r.c.core.phase() == SessionPhase::Running);
    catalog.failure.store(true, Ordering::SeqCst);
    rig.request(AppRequest::RefreshCatalog);
    rig.wait(|r| r.c.catalog_error.is_some());
    assert!(Arc::ptr_eq(&rows, &rig.c.catalog));
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    assert_eq!(rig.c.core.error(), None);
    catalog.failure.store(false, Ordering::SeqCst);
    rig.request(AppRequest::RefreshCatalog);
    rig.wait(|r| r.c.catalog_error.is_none());
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_signal_demand_still_applies_controls_and_retires_across_owner_swaps() {
    let mut r = Rig::new();
    let source = r.c.signals.topics.tone_output.clone();
    r.start();
    r.request(AppRequest::SetGainPan {
        gain: 0.008,
        pan: 0.5,
    });
    r.wait(|r| r.c.control.accepted_pending == 0 && r.c.control.last_result.is_some());
    assert_eq!(source.business_subscribers(), 0);
    assert_eq!(r.c.levels.demanded_levels(), 0);
    // A transport-level change swaps the owner: the level registry retires
    // with the old owner and rebinds cleanly on the replacement.
    let generation = r.c.core.generation();
    let mut settings = AudioSettings::default();
    settings.processing.block_frames = 512;
    r.request(AppRequest::SetAudioSettings(settings));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && r.c.core.generation() != generation);
    assert_eq!(r.c.levels.demanded_levels(), 0);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    assert_eq!(source.business_subscribers(), 0);
}

#[test]
fn recording_requests_run_pause_and_finalize_through_the_service_domain() {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    rig.wait(|r| {
        let graph = r.c.graph.snapshot_ref();
        graph.preparing_nodes.is_empty() && graph.active_revision.is_some()
    });
    let recorder = rig.node(NodeKind::Recorder);
    let directory = std::env::temp_dir();

    rig.request(AppRequest::StartRecording {
        node: recorder,
        directory: directory.clone(),
        format: crate::service::RecordingFormat::Wav,
        mp3: Default::default(),
        flac: Default::default(),
    });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording));
    // A second start while live is ignored; the first file keeps running.
    rig.request(AppRequest::StartRecording {
        node: recorder,
        directory,
        format: crate::service::RecordingFormat::Wav,
        mp3: Default::default(),
        flac: Default::default(),
    });
    rig.tick();
    assert_eq!(
        rig.c.recordings.phase(recorder),
        Some(RecordingPhase::Recording)
    );

    rig.request(AppRequest::PauseRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Paused));
    rig.request(AppRequest::ResumeRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording));
    rig.request(AppRequest::StopRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Finished));

    let snapshot = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .expect("a terminal recording keeps its entry");
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert!(
        snapshot.file_name.starts_with("moiren-rec-") && !snapshot.file_name.contains(' '),
        "unexpected file name: {}",
        snapshot.file_name
    );
    assert!(
        snapshot.path.exists(),
        "{} was not finalized",
        snapshot.path.display()
    );
    let _ = std::fs::remove_file(&snapshot.path);
}

#[test]
fn recording_survives_owner_swaps_and_continues_the_same_file() {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    rig.wait(|r| {
        let graph = r.c.graph.snapshot_ref();
        graph.preparing_nodes.is_empty() && graph.active_revision.is_some()
    });
    let recorder = rig.node(NodeKind::Recorder);
    rig.request(AppRequest::StartRecording {
        node: recorder,
        directory: std::env::temp_dir(),
        format: crate::service::RecordingFormat::Wav,
        mp3: Default::default(),
        flac: Default::default(),
    });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording));
    let first = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .unwrap();
    // Tape a first span from the initial owner. Anchoring on written-frame
    // growth instead of recorded seconds keeps the test honest on loaded
    // runners where the realtime renderer is starved.
    let first_frames = wait_written_frames(&mut rig, recorder, 0);

    // A transport-level change swaps the owner: the session parks on its
    // open file (Rebinding) and continues automatically once the
    // replacement owner runs.
    let generation = rig.c.core.generation();
    let mut settings = AudioSettings::default();
    settings.processing.block_frames = 512;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.core.generation() != generation
            && r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording)
    });

    // The file is the same one and keeps growing across the swap.
    let resumed = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .unwrap();
    assert_eq!(resumed.path, first.path);
    assert_eq!(resumed.file_name, first.file_name);
    wait_written_frames(&mut rig, recorder, first_frames);

    rig.request(AppRequest::StopRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Finished));
    let snapshot = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .expect("a terminal recording keeps its entry");
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert!(snapshot.path.exists(), "{}", snapshot.path.display());
    let _ = std::fs::remove_file(&snapshot.path);
}

#[test]
fn mp3_recording_start_finalizes_a_playable_file_through_the_service_domain() {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    rig.wait(|r| {
        let graph = r.c.graph.snapshot_ref();
        graph.preparing_nodes.is_empty() && graph.active_revision.is_some()
    });
    let recorder = rig.node(NodeKind::Recorder);
    rig.request(AppRequest::StartRecording {
        node: recorder,
        directory: std::env::temp_dir(),
        format: crate::service::RecordingFormat::Mp3,
        // Deliberately off-default knobs prove the settings ride the
        // request into the encoder, not just the default path.
        mp3: moiren_engine::recorder::Mp3Encoding {
            bitrate_kbps: 192,
            mode: moiren_engine::recorder::Mp3Mode::Vbr { quality: 4 },
        },
        flac: Default::default(),
    });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording));
    rig.request(AppRequest::StopRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Finished));
    let snapshot = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .expect("a terminal recording keeps its entry");
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert!(
        snapshot.file_name.ends_with(".mp3"),
        "{}",
        snapshot.file_name
    );
    assert!(
        snapshot.path.exists(),
        "{} missing",
        snapshot.path.display()
    );
    let bytes = std::fs::read(&snapshot.path).unwrap();
    let tagged = bytes.len() >= 3 && &bytes[..3] == b"ID3";
    let synced = bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0;
    assert!(tagged || synced, "not an MP3 stream");
    let _ = std::fs::remove_file(&snapshot.path);
}

#[test]
fn flac_recording_start_finalizes_a_playable_file_through_the_service_domain() {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    rig.wait(|r| {
        let graph = r.c.graph.snapshot_ref();
        graph.preparing_nodes.is_empty() && graph.active_revision.is_some()
    });
    let recorder = rig.node(NodeKind::Recorder);
    rig.request(AppRequest::StartRecording {
        node: recorder,
        directory: std::env::temp_dir(),
        format: crate::service::RecordingFormat::Flac,
        mp3: Default::default(),
        // An off-default level proves the setting rides the request into the
        // encoder; level 0 also exercises the smallest block preset.
        flac: moiren_engine::recorder::FlacEncoding { level: 0 },
    });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Recording));
    rig.request(AppRequest::StopRecording { node: recorder });
    rig.wait(|r| r.c.recordings.phase(recorder) == Some(RecordingPhase::Finished));
    let snapshot = rig
        .c
        .recordings
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.node == recorder)
        .expect("a terminal recording keeps its entry");
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert!(
        snapshot.file_name.ends_with(".flac"),
        "{}",
        snapshot.file_name
    );
    // The file opens with the FLAC stream marker; decode-level verification
    // lives with the engine sink's own round-trip tests.
    let bytes = std::fs::read(&snapshot.path).unwrap();
    assert!(bytes.starts_with(b"fLaC"), "not a FLAC stream");
    let _ = std::fs::remove_file(&snapshot.path);
}
