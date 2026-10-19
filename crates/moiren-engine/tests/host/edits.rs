use super::support::*;
use moiren_core::graph::{NodeKind, SendParams, edit::connect_to_new_bus_input};
use moiren_engine::host::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[test]
fn edits_return_created_node_port_and_edge_identities_after_acceptance() {
    let (project, input, _, _) = project();
    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let receipt = host
        .control()
        .edit_with_result(move |project| {
            let bus = project.graph.create_node(NodeKind::Bus, 1)?;
            project.nodes.insert(bus, NodeConfig::Bus);
            let edge =
                connect_to_new_bus_input(&mut project.graph, source, bus, SendParams::default())?;
            let input = project.graph.get_edge(edge)?.dst_port();
            Ok((bus, input, edge))
        })
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    let (bus, input, edge) = receipt.result().unwrap();
    let accepted = host.observer().read();
    let edge = accepted.desired.graph.get_edge(edge).unwrap();
    assert_eq!(edge.dst(), bus);
    assert_eq!(edge.dst_port(), input);
    assert_eq!(edge.src_port(), source);
}

#[test]
fn edits_do_not_publish_results_from_invalid_or_over_capacity_candidates() {
    let host = EngineHost::<f64>::open(
        HostConfig {
            max_nodes: 1,
            ..Default::default()
        },
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let invalid = host
        .control()
        .edit_with_result(|project| Ok(project.graph.create_node(NodeKind::Gain, 1)?))
        .unwrap();
    assert!(matches!(
        invalid.wait(TIMEOUT),
        EditOutcome::Rejected(ProjectError::MissingConfiguration(_))
    ));
    assert!(invalid.result().is_none());
    let over_capacity = host
        .control()
        .edit_with_result(|project| {
            for _ in 0..2 {
                let bus = project.graph.create_node(NodeKind::Bus, 1)?;
                project.nodes.insert(bus, NodeConfig::Bus);
            }
            Ok(123)
        })
        .unwrap();
    assert_eq!(
        over_capacity.wait(TIMEOUT),
        EditOutcome::Rejected(ProjectError::Capacity)
    );
    assert!(over_capacity.result().is_none());
    let project = host.observer().read();
    assert_eq!(project.state.desired_revision, 1);
    assert!(project.desired.graph.nodes().is_empty());
}

#[test]
fn edits_reject_same_basis_in_one_command_drain_without_running_stale_callback() {
    let host = EngineHost::<f64>::open(
        HostConfig::default(),
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let control = host.control();
    let basis = host.observer().read().edit_basis();
    let (entered, started) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let first = control
        .edit_if_current(basis, move |project| {
            entered.send(()).unwrap();
            gate.recv_timeout(TIMEOUT).unwrap();
            let bus = project.graph.create_node(NodeKind::Bus, 1)?;
            project.nodes.insert(bus, NodeConfig::Bus);
            Ok(bus)
        })
        .unwrap();
    started.recv_timeout(TIMEOUT).unwrap();
    assert!(first.result().is_none());
    let called = Arc::new(AtomicBool::new(false));
    let stale_called = called.clone();
    let second = control
        .edit_if_current(basis, move |_| {
            stale_called.store(true, Ordering::SeqCst);
            Ok(456)
        })
        .unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        first.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    assert_eq!(
        second.wait(TIMEOUT),
        EditOutcome::Conflict {
            expected: basis,
            actual: EditBasis {
                graph_revision: 2,
                ..basis
            },
        }
    );
    assert!(!called.load(Ordering::SeqCst));
    assert!(second.result().is_none());
    assert_eq!(host.observer().read().desired.graph.nodes().len(), 1);
}

#[test]
fn edits_reject_foreign_host_even_when_graph_revisions_match() {
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
    let expected = first.observer().read().edit_basis();
    let actual = second.observer().read().edit_basis();
    let receipt = second
        .control()
        .edit_if_current(expected, |_| -> Result<(), ProjectError> {
            panic!("foreign edit must not run");
        })
        .unwrap();
    assert_eq!(
        receipt.wait(TIMEOUT),
        EditOutcome::Conflict { expected, actual }
    );
    assert!(receipt.result().is_none());
    assert!(second.observer().read().state.fault.is_none());
}

#[test]
fn edits_keep_results_of_accepted_edits_when_preparation_is_superseded() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project().0, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |state| state.active_revision == Some(1));
    state.block_update.store(true, Ordering::SeqCst);
    let add_bus = |project: &mut EngineProject| {
        let bus = project.graph.create_node(NodeKind::Bus, 1)?;
        project.nodes.insert(bus, NodeConfig::Bus);
        Ok(bus)
    };
    let first = host.control().edit_with_result(add_bus).unwrap();
    wait_count(&state.prepares, 2);
    let first_id = first.result().unwrap();
    let second = host.control().edit_with_result(add_bus).unwrap();
    assert!(matches!(
        first.wait(TIMEOUT),
        EditOutcome::Superseded {
            by_graph_revision: 3
        }
    ));
    assert_eq!(first.result(), Some(first_id));
    wait(&observer, |state| state.desired_revision == 3);
    assert!(observer.read().desired.graph.get_node(first_id).is_ok());
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(
        second.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 3,
            ..
        }
    ));
    assert_ne!(first.result(), second.result());
}

#[test]
fn edits_panicked_callback_has_no_result_and_does_not_publish_partial_project() {
    let host = EngineHost::<f64>::open(
        HostConfig::default(),
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let observer = host.observer();
    let receipt = host
        .control()
        .edit_with_result(|project| -> Result<u8, ProjectError> {
            let bus = project.graph.create_node(NodeKind::Bus, 1)?;
            project.nodes.insert(bus, NodeConfig::Bus);
            panic!("injected edit panic");
        })
        .unwrap();
    assert!(matches!(receipt.wait(TIMEOUT), EditOutcome::Rejected(_)));
    assert!(receipt.result().is_none());
    drop(host);
    assert!(observer.read().desired.graph.nodes().is_empty());
    assert!(receipt.result().is_none());
}

#[test]
fn edits_typed_receipt_clone_shares_capacity_and_survives_owner_close() {
    struct MoveOnly;
    let host = EngineHost::<f64>::open(
        HostConfig {
            receipt_capacity: 1,
            ..Default::default()
        },
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let control = host.control();
    let receipt = control.edit_with_result(|_| Ok(MoveOnly)).unwrap();
    let retained = receipt.clone();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Activated { .. }
    ));
    assert!(matches!(control.edit(|_| Ok(())), Err(HostError::Capacity)));
    drop(receipt);
    assert!(matches!(control.edit(|_| Ok(())), Err(HostError::Capacity)));
    drop(retained);
    // The coordinator may still own its receipt after waking the waiting caller.
    let next = wait_until(
        "coordinator did not release the completed receipt",
        TIMEOUT,
        || match control.edit_with_result(|_| Ok(789)) {
            Ok(receipt) => Some(receipt),
            Err(HostError::Capacity) => None,
            Err(error) => panic!("next edit failed: {error}"),
        },
    );
    assert!(matches!(next.wait(TIMEOUT), EditOutcome::Activated { .. }));
    drop(host);
    assert_eq!(next.result(), Some(789));
    assert!(matches!(control.edit(|_| Ok(())), Err(HostError::Closed)));
}
