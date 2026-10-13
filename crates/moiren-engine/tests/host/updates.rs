use super::support::*;
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::host::*;
use std::{sync::atomic::Ordering, time::Duration};

#[test]
fn updates_reuse_io_and_keep_owner_epoch_and_active_confirmation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    let initial = wait(&observer, |snapshot| snapshot.active_revision == Some(1));
    state.block_update.store(true, Ordering::SeqCst);
    let edit = host
        .control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })
        .unwrap();
    wait_count(&state.prepares, 2);
    let preparing = wait(&observer, |snapshot| snapshot.desired_revision == 2);
    assert_eq!(preparing.active_revision, Some(1));
    assert_eq!(preparing.desired_revision, 2);
    assert!(matches!(edit.outcome(), EditOutcome::Accepted { .. }));
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(
        edit.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    let active = wait(&observer, |snapshot| {
        snapshot.active_revision == Some(2) && snapshot.timeline.is_some()
    });
    assert_eq!(active.owner_generation, initial.owner_generation);
    assert_eq!(
        active.timeline.unwrap().epoch,
        initial.timeline.unwrap().epoch
    );
    assert!(active.timeline.unwrap().frame >= initial.timeline.unwrap().frame);
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(state.sources.load(Ordering::SeqCst), 1);
    assert_eq!(state.sinks.load(Ordering::SeqCst), 1);
    wait_sample(&state, 0.5);
}

#[test]
fn updates_coalesce_edits_and_discard_stale_preparation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.active_revision == Some(1)
    });
    state.block_update.store(true, Ordering::SeqCst);
    state.ignore_update_cancel.store(true, Ordering::SeqCst);
    let first = host
        .control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.2 });
            Ok(())
        })
        .unwrap();
    wait_count(&state.prepares, 2);
    let second = host
        .control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.3 });
            Ok(())
        })
        .unwrap();
    let third = host
        .control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.4 });
            Ok(())
        })
        .unwrap();
    wait(&host.observer(), |snapshot| snapshot.desired_revision == 4);
    assert!(matches!(first.outcome(), EditOutcome::Superseded { .. }));
    assert!(matches!(second.outcome(), EditOutcome::Superseded { .. }));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(
        third.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 4,
            ..
        }
    ));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 3);
    wait_sample(&state, 0.4);
}

#[test]
fn updates_failure_preserves_desired_and_running_basis_without_retry_loop() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.active_revision == Some(1)
    });
    state.fail_update.store(true, Ordering::SeqCst);
    host.control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })
        .unwrap();
    wait(&host.observer(), |snapshot| {
        matches!(
            snapshot.update,
            UpdateState::Blocked {
                graph_revision: 2,
                ..
            }
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
    assert_eq!(host.observer().snapshot().active_revision, Some(1));
    assert_eq!(
        host.observer().project().nodes.get(&gain),
        Some(&NodeConfig::Gain { initial: 0.5 })
    );
    state.fail_update.store(false, Ordering::SeqCst);
    let edit = host
        .control()
        .edit(move |project| {
            project.processing.audio_byte_budget += 8;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        edit.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 3,
            ..
        }
    ));
}

#[test]
fn updates_invalid_transaction_does_not_change_desired_or_active() {
    let host = EngineHost::<f64>::open(HostConfig::default(), project().0, FakeBackend::default())
        .unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.active_revision == Some(1)
    });
    let original = host.observer().project();
    let receipt = host
        .control()
        .edit(|project| {
            project.graph.create_node(NodeKind::Gain, 1)?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Rejected(ProjectError::MissingConfiguration(_))
    ));
    assert_eq!(host.observer().project(), original);
    assert_eq!(host.observer().snapshot().desired_revision, 1);
}

#[test]
fn updates_edge_values_route_without_graph_revision_or_recompile() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, _, _) = project();
    let edge = project.graph.edges()[0].id();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.active_revision == Some(1)
    });
    let edit = host
        .control()
        .edit(move |project| {
            project.graph.set_send_params(
                edge,
                SendParams {
                    gain: 0.25,
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        edit.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 1,
            ..
        }
    ));
    wait_sample(&state, 0.25);
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_blocked_preparation_does_not_hold_owner_cleanup_lane() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| snapshot.active_revision == Some(1));
    state.block_update.store(true, Ordering::SeqCst);
    state.ignore_update_cancel.store(true, Ordering::SeqCst);
    host.control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })
        .unwrap();
    wait_count(&state.prepares, 2);
    let close = std::thread::spawn(move || drop(host));
    wait_count(&state.stops, 1);
    wait(&observer, |snapshot| snapshot.host == HostState::Closing);
    assert!(!close.is_finished());
    state.block_update.store(false, Ordering::SeqCst);
    close.join().unwrap();
    assert_eq!(observer.snapshot().host, HostState::Closed);
}
