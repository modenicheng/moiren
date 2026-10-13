use super::support::*;
use moiren_core::{
    graph::{NodeKind, SendParams},
    protocol::ParamValue,
};
use moiren_engine::{host::*, processor::Gain};
use std::sync::atomic::Ordering;

#[test]
fn parameters_apply_to_stable_node_without_recompile_or_graph_revision() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let receipt = host
        .control()
        .parameter(
            ParameterTarget::Node {
                node: gain,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.25),
            ParameterTiming::NextBlock,
            8,
        )
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        ParameterOutcome::Applied(_)
    ));
    wait_sample(&state, 0.25);
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(host.observer().snapshot().desired_revision, 1);
}

#[test]
fn parameters_new_target_waits_for_confirmed_mapping_and_reconciles_initial_value() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, input, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    state.block_update.store(true, Ordering::SeqCst);
    let (created, node) = std::sync::mpsc::channel();
    let edit = host
        .control()
        .edit(move |project| {
            let edge = project
                .graph
                .edges()
                .iter()
                .find(|edge| edge.dst() == gain)
                .unwrap()
                .id();
            project.graph.disconnect(edge)?;
            let added = project.graph.create_node(NodeKind::Gain, 1)?;
            project
                .nodes
                .insert(added, NodeConfig::Gain { initial: 1.0 });
            let source = project.graph.get_node(input)?.outputs()[0].id();
            let added_input = project.graph.get_node(added)?.inputs()[0].id();
            let added_output = project.graph.get_node(added)?.outputs()[0].id();
            let destination = project.graph.get_node(gain)?.inputs()[0].id();
            project
                .graph
                .connect(source, added_input, SendParams::default())?;
            project
                .graph
                .connect(added_output, destination, SendParams::default())?;
            created.send(added).unwrap();
            Ok(())
        })
        .unwrap();
    let added = node.recv_timeout(TIMEOUT).unwrap();
    wait_count(&state.prepares, 2);
    let parameter = host
        .control()
        .parameter(
            ParameterTarget::Node {
                node: added,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.5),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    wait(&host.observer(), |snapshot| snapshot.desired_revision == 2);
    assert_eq!(
        wait_parameter_consumed(&parameter),
        ParameterOutcome::Accepted
    );
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    assert!(matches!(
        parameter.wait(TIMEOUT),
        ParameterOutcome::Applied(_)
    ));
    wait_sample(&state, 0.5);
    let stable = host
        .control()
        .parameter(
            ParameterTarget::Node {
                node: gain,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.8),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    assert!(matches!(stable.wait(TIMEOUT), ParameterOutcome::Applied(_)));
    wait_sample(&state, 0.4);
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn parameters_reject_invalid_domain_and_old_epoch_without_changing_desired() {
    let (project, _, gain, _) = project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let snapshot = wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let target = ParameterTarget::Node {
        node: gain,
        parameter: Gain::LEVEL,
    };
    let invalid = host
        .control()
        .parameter(
            target,
            ParamValue::Float(17.0),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    assert_eq!(
        invalid.wait(TIMEOUT),
        ParameterOutcome::Rejected(ParameterFailure::InvalidValue)
    );
    let stale = host
        .control()
        .parameter(
            target,
            ParamValue::Float(0.5),
            ParameterTiming::Frame {
                epoch: snapshot.timeline.unwrap().epoch + 1,
                frame: 100,
            },
            0,
        )
        .unwrap();
    assert_eq!(
        stale.wait(TIMEOUT),
        ParameterOutcome::Rejected(ParameterFailure::StaleEpoch)
    );
    assert!(host.observer().project().parameters.is_empty());
}

#[test]
fn parameters_accepted_future_request_terminates_on_host_drop() {
    let (project, _, gain, _) = project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let snapshot = wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let timeline = snapshot.timeline.unwrap();
    state.pause_audio.store(true, Ordering::SeqCst);
    let receipt = host
        .control()
        .parameter(
            ParameterTarget::Node {
                node: gain,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.5),
            ParameterTiming::Frame {
                epoch: timeline.epoch,
                frame: timeline.frame + 40_000,
            },
            0,
        )
        .unwrap();
    assert_eq!(
        wait_parameter_consumed(&receipt),
        ParameterOutcome::Accepted
    );
    drop(host);
    assert_eq!(receipt.outcome(), ParameterOutcome::Closed);
}
