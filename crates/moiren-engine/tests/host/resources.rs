use super::support::*;
use moiren_engine::host::*;
use std::{sync::atomic::Ordering, time::Duration};

#[test]
fn resources_wait_recover_and_ignore_duplicate_or_unrelated_events() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    state.unavailable.store(true, Ordering::SeqCst);
    let (project, _, gain, output) = project();
    let configuration = project.nodes[&output].clone();
    let unrelated = project.nodes[&gain].clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::WaitingForResources
    });
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(host.observer().snapshot().desired_revision, 1);
    state.unavailable.store(false, Ordering::SeqCst);
    state.events.lock().unwrap().push_back(ResourceEvent {
        node: output,
        configuration: configuration.clone(),
        generation: 1,
        available: true,
    });
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
    state.events.lock().unwrap().extend([
        ResourceEvent {
            node: output,
            configuration,
            generation: 1,
            available: false,
        },
        ResourceEvent {
            node: gain,
            configuration: unrelated,
            generation: 99,
            available: false,
        },
    ]);
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(state.stops.load(Ordering::SeqCst), 0);
}

#[test]
fn resources_lost_owner_is_reclaimed_and_recovers_in_new_epoch() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, _, output) = project();
    let configuration = project.nodes[&output].clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let before = wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    state.fail_owner.store(true, Ordering::SeqCst);
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::WaitingForResources
            && snapshot.active_revision.is_none()
    });
    wait_count(&state.stops, 1);
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
    state.fail_owner.store(false, Ordering::SeqCst);
    state.events.lock().unwrap().push_back(ResourceEvent {
        node: output,
        configuration,
        generation: 2,
        available: true,
    });
    let recovered = wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
            && snapshot.owner_generation > before.owner_generation
    });
    assert!(recovered.timeline.unwrap().epoch > before.timeline.unwrap().epoch);
    assert_eq!(recovered.active_revision, Some(1));
}

#[test]
fn resources_removing_output_releases_owner_and_keeps_project_dormant() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, _, output) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let receipt = host
        .control()
        .edit(move |project| {
            project.graph.remove_node(output)?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            plan_revision: None
        }
    ));
    assert_eq!(
        host.observer().snapshot().execution,
        ExecutionState::Dormant
    );
    assert!(!host.observer().project().nodes.contains_key(&output));
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn resources_changing_an_unavailable_selection_ignores_old_configuration_events() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    state.unavailable.store(true, Ordering::SeqCst);
    let (project, _, _, output) = project();
    let configuration = project.nodes[&output].clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::WaitingForResources
    });
    state.events.lock().unwrap().push_back(ResourceEvent {
        node: output,
        configuration: configuration.clone(),
        generation: 1,
        available: false,
    });
    wait(&host.observer(), |snapshot| {
        snapshot.resource_generation == 1
    });
    state.unavailable.store(false, Ordering::SeqCst);
    let stale_events = state.clone();
    let receipt = host
        .control()
        .edit(move |project| {
            let Some(NodeConfig::Output(config)) = project.nodes.get_mut(&output) else {
                unreachable!();
            };
            config.target = moiren_engine::node::OutputTarget::Software {
                name: "replacement".into(),
            };
            stale_events
                .events
                .lock()
                .unwrap()
                .push_back(ResourceEvent {
                    node: output,
                    configuration,
                    generation: 99,
                    available: false,
                });
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    let snapshot = wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    assert_eq!(snapshot.resource_generation, 1);
    assert_eq!(snapshot.active_revision, Some(2));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
}
