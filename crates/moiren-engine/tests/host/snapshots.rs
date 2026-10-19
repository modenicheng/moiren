use super::support::*;
use moiren_engine::host::*;
use std::sync::{Arc, atomic::Ordering};

#[test]
fn snapshots_keep_desired_and_active_projects_separate_during_update() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |state| state.active_revision == Some(1));
    let before = observer.read();
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
    wait(&observer, |state| state.desired_revision == 2);
    let pending = observer.read();
    assert_eq!(pending.state.active_revision, Some(1));
    assert_eq!(
        pending.desired.nodes[&gain],
        NodeConfig::Gain { initial: 0.5 }
    );
    assert_eq!(
        pending.active.as_ref().unwrap().nodes[&gain],
        NodeConfig::Gain { initial: 1.0 }
    );
    assert_eq!(
        before.desired.nodes[&gain],
        NodeConfig::Gain { initial: 1.0 }
    );
    assert!(Arc::ptr_eq(
        pending.active.as_ref().unwrap(),
        before.active.as_ref().unwrap()
    ));
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(
        edit.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    let applied = observer.read();
    assert_eq!(applied.state.active_revision, Some(2));
    assert_eq!(
        applied.active.as_ref().unwrap().nodes[&gain],
        NodeConfig::Gain { initial: 0.5 }
    );
    drop(host);
    let closed = observer.read();
    assert_eq!(closed.state.host, HostState::Closed);
    assert!(closed.active.is_none());
    assert_eq!(closed.state.active_revision, None);
    assert_eq!(
        closed.desired.nodes[&gain],
        NodeConfig::Gain { initial: 0.5 }
    );
}

#[test]
fn snapshots_reuse_immutable_projects_and_distinguish_hosts() {
    let first = EngineHost::<f64>::open(
        HostConfig::default(),
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let second = EngineHost::<f64>::open(
        HostConfig::default(),
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let observer = first.observer();
    wait(&observer, |state| state.host == HostState::Open);
    let before = observer.read();
    let no_change = first.control().edit(|_| Ok(())).unwrap();
    assert!(matches!(
        no_change.wait(TIMEOUT),
        EditOutcome::Activated { .. }
    ));
    let after = observer.read();
    assert!(Arc::ptr_eq(&before.desired, &after.desired));
    assert_ne!(after.host_id, second.observer().read().host_id);
    assert!(after.active.is_none());
}

#[test]
fn snapshots_publish_matching_project_and_revision_under_concurrent_edits() {
    let (project, _, gain, _) = project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |state| state.active_revision == Some(1));
    let control = host.control();
    let writer = std::thread::spawn(move || {
        (2..=33)
            .map(|revision| {
                control
                    .edit(move |project| {
                        project.nodes.insert(
                            gain,
                            NodeConfig::Gain {
                                initial: revision as f64 * 0.01,
                            },
                        );
                        Ok(())
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>()
    });
    wait_until("desired revision did not reach 33", TIMEOUT, || {
        let snapshot = observer.read();
        let expected = |revision| {
            if revision == 1 {
                1.0
            } else {
                revision as f64 * 0.01
            }
        };
        assert_eq!(
            snapshot.desired.nodes[&gain],
            NodeConfig::Gain {
                initial: expected(snapshot.state.desired_revision)
            }
        );
        if let Some(revision) = snapshot.state.active_revision {
            assert_eq!(
                snapshot.active.as_ref().unwrap().nodes[&gain],
                NodeConfig::Gain {
                    initial: expected(revision)
                }
            );
        } else {
            assert!(snapshot.active.is_none());
        }
        (snapshot.state.desired_revision == 33).then_some(())
    });
    writer.join().unwrap();
}

#[test]
fn snapshots_refresh_desired_parameters_without_advancing_graph_revision() {
    use moiren_core::protocol::ParamValue;
    use moiren_engine::processor::Gain;
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |state| {
        state.active_revision == Some(1) && state.timeline.is_some()
    });
    let before = observer.read();
    let target = ParameterTarget::Node {
        node: gain,
        parameter: Gain::LEVEL,
    };
    for (value, timing) in [
        (0.25, ParameterTiming::NextBlock),
        (0.4, {
            let timeline = before.state.timeline.unwrap();
            ParameterTiming::Frame {
                epoch: timeline.epoch,
                frame: timeline.frame + 128,
            }
        }),
    ] {
        let receipt = host
            .control()
            .parameter(target, ParamValue::Float(value), timing, 0)
            .unwrap();
        assert!(matches!(
            receipt.wait(TIMEOUT),
            ParameterOutcome::Applied(_)
        ));
        wait_until("desired parameter values did not refresh", TIMEOUT, || {
            let snapshot = observer.read();
            if snapshot.desired.parameter_values().iter().any(|parameter| {
                parameter.target == target && parameter.value == ParamValue::Float(value)
            }) {
                assert_eq!(snapshot.state.desired_revision, 1);
                assert_eq!(snapshot.state.active_revision, Some(1));
                assert!(!Arc::ptr_eq(&snapshot.desired, &before.desired));
                assert!(Arc::ptr_eq(
                    snapshot.active.as_ref().unwrap(),
                    before.active.as_ref().unwrap()
                ));
                Some(())
            } else {
                None
            }
        });
        wait_sample(&state, value);
    }
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
}
