use super::*;
use crate::service::{GraphEdit, ServiceGraph};

pub(super) fn intent(graph: &ServiceGraph) -> ExportIntent {
    ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([0x81; 16]),
        direction: ExportDirection::Playback,
        name: "Application playback".into(),
        channels: 2,
        enabled: true,
        binding: graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .map(|node| node.id()),
    }
}

#[test]
fn native_name_limits_are_validated_before_applying_an_export() {
    for invalid in [
        "".into(),
        " ".into(),
        "line\nbreak".into(),
        "x".repeat(64),
        "\u{1f3b5}".repeat(32),
    ] {
        assert!(validate_export_name_channels(&invalid, 2).is_err());
    }
    assert!(validate_export_name_channels(&"x".repeat(63), 2).is_ok());
    assert!(validate_export_name_channels(&"\u{1f3b5}".repeat(31), 1).is_ok());
}

#[test]
fn enabled_intent_plans_creation_without_granting_default_or_claiming_activity() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let mut service = ExportService::default();
    let snapshot = graph.snapshot();
    assert!(!service.reconcile(&snapshot).available);
    assert!(service.reconcile(&snapshot).actions.is_empty());
    assert_eq!(
        service.statuses(&snapshot)[0].resource,
        ExportResourceState::Unavailable
    );
    service.set_available(true);
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Create {
            intent: intent.clone(),
            allow_default: false,
        }]
    );
    assert_eq!(
        service.statuses(&snapshot)[0].resource,
        ExportResourceState::Absent
    );
    assert!(!service.statuses(&snapshot)[0].gate_open);
    service
        .set_authorizations(&[ExportAuthorization {
            endpoint_id: intent.endpoint_id,
            allow_default: true,
        }])
        .unwrap();
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Create {
            intent,
            allow_default: true,
        }]
    );
}

#[test]
fn microphone_and_disabled_intents_use_their_declared_shape() {
    let mut graph = ServiceGraph::default();
    let sink = graph
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Sink)
        .map(|node| node.id())
        .unwrap();
    let microphone = ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([0x82; 16]),
        direction: ExportDirection::Microphone,
        name: "Virtual microphone".into(),
        channels: 2,
        enabled: true,
        binding: Some(sink),
    };
    graph
        .edit(GraphEdit::SetExports(vec![microphone.clone()]))
        .unwrap();
    assert_eq!(graph.snapshot().exports, vec![microphone]);

    let disabled = ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([0x83; 16]),
        direction: ExportDirection::Playback,
        name: "Disabled playback".into(),
        channels: 1,
        enabled: false,
        binding: None,
    };
    graph
        .edit(GraphEdit::SetExports(vec![disabled.clone()]))
        .unwrap();
    let mut service = ExportService::default();
    assert!(
        service
            .preflight_audio(std::slice::from_ref(&disabled))
            .is_ok()
    );
    service.set_available(true);
    assert!(service.reconcile(&graph.snapshot()).actions.is_empty());
    assert_eq!(graph.snapshot().exports, vec![disabled]);
}

#[test]
fn lifecycle_gate_and_engine_confirmation_are_independent_and_stale_reports_are_rejected() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let mut snapshot = graph.snapshot();
    let mut service = ExportService::default();
    service.set_available(true);
    let mut observation = ExportObservation {
        intent,
        generation: 1,
        resource: ExportResourceState::Creating,
        allow_default: false,
        gate_open: false,
        active_revision: None,
    };
    service.observe(observation.clone()).unwrap();
    assert!(service.reconcile(&snapshot).actions.is_empty());
    observation.generation = 2;
    observation.resource = ExportResourceState::Present;
    service.observe(observation.clone()).unwrap();
    assert!(service.reconcile(&snapshot).actions.is_empty());
    assert_eq!(service.statuses(&snapshot)[0].active_revision, None);
    observation.generation = 3;
    observation.active_revision = Some(snapshot.candidate_revision);
    snapshot.active_revision = observation.active_revision;
    service.observe(observation.clone()).unwrap();
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::SetGate {
            endpoint_id: observation.intent.endpoint_id,
            open: true,
        }]
    );
    assert!(
        !service.statuses(&snapshot)[0].gate_open,
        "planning cannot claim completion"
    );
    assert!(service.observe(observation.clone()).is_err());
    observation.generation = 4;
    observation.gate_open = true;
    service.observe(observation.clone()).unwrap();
    snapshot.exports.clear();
    snapshot.candidate_revision += 1;
    assert!(
        service.reconcile(&snapshot).actions.is_empty(),
        "an unapplied candidate cannot destroy active resources"
    );
    snapshot.active_revision = None;
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::SetGate {
            endpoint_id: observation.intent.endpoint_id,
            open: false,
        }]
    );
    observation.generation = 5;
    observation.gate_open = false;
    observation.active_revision = None;
    service.observe(observation.clone()).unwrap();
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Remove {
            endpoint_id: observation.intent.endpoint_id,
        }]
    );
    observation.generation = 6;
    observation.resource = ExportResourceState::Removing;
    service.observe(observation).unwrap();
    assert!(service.reconcile(&snapshot).actions.is_empty());
}

#[test]
fn stale_creating_export_is_removed_instead_of_left_as_a_ghost_endpoint() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .observe(ExportObservation {
            intent: intent.clone(),
            generation: 1,
            resource: ExportResourceState::Creating,
            allow_default: false,
            gate_open: false,
            active_revision: None,
        })
        .unwrap();

    graph.edit(GraphEdit::SetExports(Vec::new())).unwrap();

    assert_eq!(
        service.reconcile(&graph.snapshot()).actions,
        vec![ExportAction::Remove {
            endpoint_id: intent.endpoint_id,
        }]
    );
}

#[test]
fn removed_export_is_recreated_only_after_removal_is_confirmed() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let snapshot = graph.snapshot();
    let mut service = ExportService::default();
    service.set_available(true);
    let mut observation = ExportObservation {
        intent: intent.clone(),
        generation: 1,
        resource: ExportResourceState::Removing,
        allow_default: false,
        gate_open: false,
        active_revision: None,
    };
    service.observe(observation.clone()).unwrap();
    assert!(service.reconcile(&snapshot).actions.is_empty());

    observation.generation = 2;
    // Absent is published only once asynchronous driver removal is confirmed.
    observation.resource = ExportResourceState::Absent;
    service.observe(observation).unwrap();

    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Create {
            intent,
            allow_default: false,
        }]
    );
}

#[test]
fn default_eligibility_change_closes_gate_before_rebuilding_endpoint() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let snapshot = graph.snapshot();
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .set_authorizations(&[ExportAuthorization {
            endpoint_id: intent.endpoint_id,
            allow_default: true,
        }])
        .unwrap();
    let mut observation = ExportObservation {
        intent: intent.clone(),
        generation: 1,
        resource: ExportResourceState::Present,
        allow_default: false,
        gate_open: true,
        active_revision: Some(snapshot.candidate_revision),
    };
    service.observe(observation.clone()).unwrap();

    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::SetGate {
            endpoint_id: intent.endpoint_id,
            open: false,
        }]
    );

    observation.generation = 2;
    observation.gate_open = false;
    observation.active_revision = None;
    service.observe(observation.clone()).unwrap();
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Remove {
            endpoint_id: intent.endpoint_id,
        }]
    );

    observation.generation = 3;
    observation.resource = ExportResourceState::Removing;
    service.observe(observation.clone()).unwrap();
    assert!(service.reconcile(&snapshot).actions.is_empty());

    observation.generation = 4;
    observation.resource = ExportResourceState::Absent;
    service.observe(observation).unwrap();
    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Create {
            intent,
            allow_default: true,
        }]
    );
}

#[test]
fn default_eligibility_change_retires_an_endpoint_still_being_created() {
    let mut graph = ServiceGraph::default();
    let intent = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let snapshot = graph.snapshot();
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .observe(ExportObservation {
            intent: intent.clone(),
            generation: 1,
            resource: ExportResourceState::Creating,
            allow_default: false,
            gate_open: false,
            active_revision: None,
        })
        .unwrap();
    service
        .set_authorizations(&[ExportAuthorization {
            endpoint_id: intent.endpoint_id,
            allow_default: true,
        }])
        .unwrap();

    assert_eq!(
        service.reconcile(&snapshot).actions,
        vec![ExportAction::Remove {
            endpoint_id: intent.endpoint_id,
        }]
    );
}

#[test]
fn invalid_exports_do_not_replace_candidate_or_confirmed_revision() {
    let mut graph = ServiceGraph::default();
    let good = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![good.clone()]))
        .unwrap();
    let revision = graph.snapshot().candidate_revision;
    graph.confirmed(crate::service::SessionGeneration(1), revision);
    let snapshot = graph.snapshot();
    let mut wrong_channels = good.clone();
    wrong_channels.channels = 1;
    let mut wrong_direction = good.clone();
    wrong_direction.direction = ExportDirection::Microphone;
    for values in [
        vec![good.clone(), good.clone()],
        vec![wrong_channels],
        vec![wrong_direction],
    ] {
        assert!(graph.edit(GraphEdit::SetExports(values)).is_err());
        let after = graph.snapshot();
        assert_eq!(after.active_revision, snapshot.active_revision);
        assert_eq!(after.candidate_revision, snapshot.candidate_revision);
        assert_eq!(after.exports, snapshot.exports);
    }

    let mut duplicate_binding = good;
    duplicate_binding.endpoint_id = VirtualEndpointId::from_bytes([0x84; 16]);
    assert!(
        graph
            .edit(GraphEdit::SetExports(vec![
                snapshot.exports[0].clone(),
                duplicate_binding,
            ]))
            .is_err()
    );
}

#[test]
fn unbound_export_cannot_keep_an_observed_gate_open() {
    let mut graph = ServiceGraph::default();
    let mut export = intent(&graph);
    export.binding = None;
    graph
        .edit(GraphEdit::SetExports(vec![export.clone()]))
        .unwrap();
    let revision = graph.snapshot().candidate_revision;
    graph.confirmed(crate::service::SessionGeneration(1), revision);
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .observe(ExportObservation {
            intent: export.clone(),
            generation: 1,
            resource: ExportResourceState::Present,
            allow_default: false,
            gate_open: true,
            active_revision: Some(revision),
        })
        .unwrap();

    assert_eq!(
        service.reconcile(&graph.snapshot()).actions,
        vec![ExportAction::SetGate {
            endpoint_id: export.endpoint_id,
            open: false,
        }]
    );
}

#[test]
fn candidate_unbinding_does_not_revoke_confirmed_audio_admission() {
    let mut graph = ServiceGraph::default();
    let export = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![export.clone()]))
        .unwrap();
    let revision = graph.snapshot().candidate_revision;
    graph.confirmed(crate::service::SessionGeneration(1), revision);
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .observe(ExportObservation {
            intent: export.clone(),
            generation: 1,
            resource: ExportResourceState::Present,
            allow_default: false,
            gate_open: false,
            active_revision: Some(revision),
        })
        .unwrap();
    let mut candidate = export.clone();
    candidate.binding = None;
    graph.edit(GraphEdit::SetExports(vec![candidate])).unwrap();

    assert_eq!(
        service.reconcile(&graph.snapshot()).actions,
        vec![ExportAction::SetGate {
            endpoint_id: export.endpoint_id,
            open: true,
        }]
    );
}

#[test]
fn restored_adapter_requires_fresh_engine_admission() {
    let mut graph = ServiceGraph::default();
    let export = intent(&graph);
    graph
        .edit(GraphEdit::SetExports(vec![export.clone()]))
        .unwrap();
    let revision = graph.snapshot().candidate_revision;
    graph.confirmed(crate::service::SessionGeneration(1), revision);
    let mut service = ExportService::default();
    service.set_available(true);
    service
        .observe(ExportObservation {
            intent: export.clone(),
            generation: 1,
            resource: ExportResourceState::Present,
            allow_default: false,
            gate_open: true,
            active_revision: Some(revision),
        })
        .unwrap();

    service.set_available(false);
    service.set_available(true);

    assert_eq!(
        service.reconcile(&graph.snapshot()).actions,
        vec![ExportAction::SetGate {
            endpoint_id: export.endpoint_id,
            open: false,
        }]
    );
    assert_eq!(service.statuses(&graph.snapshot())[0].active_revision, None);
}
