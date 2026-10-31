use super::*;
use crate::service::{ExportDirection, ExportIntent, GraphEdit, ServiceGraph, ServiceMailbox};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::node::VirtualEndpointId;

fn physical_graph() -> ServiceGraph {
    let mut graph = ServiceGraph::default();
    let (source, sink) = graph.preparation().primary_io_nodes().unwrap();
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
}

fn bind_virtual(graph: &mut ServiceGraph, node: NodeId, direction: ExportDirection) {
    graph.edit(GraphEdit::ClearBinding(node)).unwrap();
    let mut exports = graph.snapshot().exports;
    exports.push(ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([exports.len() as u8 + 1; 16]),
        direction,
        name: format!("Transport {:?}", direction),
        channels: 2,
        enabled: true,
        binding: Some(node),
    });
    graph.edit(GraphEdit::SetExports(exports)).unwrap();
}

fn full_catalog() -> DeviceCatalog {
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    DeviceCatalog {
        inputs: vec![row("input")],
        outputs: vec![row("output")],
        default_input_endpoint_id: Some("input".into()),
        default_output_endpoint_id: Some("output".into()),
        ..Default::default()
    }
}

fn ui(graph: ServiceGraph, physical_available: bool) -> (UiState, ServiceMailbox) {
    let mut context = crate::service::ServiceContext::default();
    context.graph = graph;
    if physical_available {
        context.catalog = Arc::new(full_catalog());
    }
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    (UiState::new(handle, initial), mailbox)
}

fn publish_snapshot_change(
    state: &mut UiState,
    mailbox: &ServiceMailbox,
    change: impl FnOnce(&mut AppSnapshot),
) {
    let mut snapshot = (*state.snapshot).clone();
    change(&mut snapshot);
    mailbox.publish(snapshot);
    state.poll();
    assert!(!state.pending_request);
}

fn publish_phase(state: &mut UiState, mailbox: &ServiceMailbox, phase: SessionPhase) {
    let mut snapshot = (*state.snapshot).clone();
    snapshot.phase = phase;
    mailbox.publish(snapshot);
    state.poll();
    assert!(!state.pending_request);
}

#[test]
fn virtual_only_transport_dispatches_apply_and_restarts_after_stop() {
    let mut graph = physical_graph();
    let (source, sink) = graph.preparation().primary_io_nodes().unwrap();
    bind_virtual(&mut graph, source, ExportDirection::Playback);
    bind_virtual(&mut graph, sink, ExportDirection::Microphone);
    let (mut state, mailbox) = ui(graph, false);
    assert!(state.candidate_spec().is_none());
    assert!(state.can_start_transport());

    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::ApplyGraph));
    publish_phase(&mut state, &mailbox, SessionPhase::Running);
    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::Stop));
    publish_phase(&mut state, &mailbox, SessionPhase::Idle);
    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::ApplyGraph));
}

#[test]
fn virtual_primary_uses_graph_bindings_not_stale_physical_selectors() {
    for direction in [ExportDirection::Playback, ExportDirection::Microphone] {
        let mut graph = physical_graph();
        let (source, sink) = graph.preparation().primary_io_nodes().unwrap();
        let node = match direction {
            ExportDirection::Playback => source,
            ExportDirection::Microphone => sink,
        };
        bind_virtual(&mut graph, node, direction);
        let (mut state, mailbox) = ui(graph, true);
        state.source_mode = SourceMode::Input;
        state.selection.input_id = Some("input".into());
        state.selection.output_id = Some("output".into());
        assert!(
            state.candidate_spec().is_some(),
            "stale physical spec exists"
        );
        assert!(state.can_start_transport());
        let before = state.snapshot.graph.clone();

        state.toggle_transport();
        assert_eq!(mailbox.try_request(), Some(AppRequest::ApplyGraph));
        assert_eq!(mailbox.try_request(), None, "no physical binding edits");
        assert_eq!(state.snapshot.graph, before);
    }
}

#[test]
fn incomplete_virtual_graph_rejects_stale_physical_fallback() {
    for direction in [ExportDirection::Playback, ExportDirection::Microphone] {
        let mut graph = physical_graph();
        let (source, sink) = graph.preparation().primary_io_nodes().unwrap();
        let (virtual_node, missing_node) = match direction {
            ExportDirection::Playback => (source, sink),
            ExportDirection::Microphone => (sink, source),
        };
        bind_virtual(&mut graph, virtual_node, direction);
        graph.edit(GraphEdit::ClearBinding(missing_node)).unwrap();
        let (mut state, mailbox) = ui(graph, true);
        state.source_mode = SourceMode::Input;
        state.selection.input_id = Some("input".into());
        state.selection.output_id = Some("output".into());
        assert!(!state.can_start_transport());

        state.toggle_transport();
        assert_eq!(mailbox.try_request(), None, "unbound graph cannot start");
        assert!(state.dispatch_error.is_some());
    }
}

#[test]
fn nonprimary_virtual_nodes_preserve_physical_start_and_run_limit() {
    let mut graph = physical_graph();
    for (kind, direction) in [
        (NodeKind::Source, ExportDirection::Playback),
        (NodeKind::Sink, ExportDirection::Microphone),
    ] {
        graph
            .edit(GraphEdit::AddNode { kind, channels: 2 })
            .unwrap();
        let node = graph.snapshot().graph.nodes().last().unwrap().id();
        bind_virtual(&mut graph, node, direction);
    }
    let (mut state, mailbox) = ui(graph, true);
    state.run_limit = RunLimit::Seconds(12);
    state.max_block_frames = 512;
    let expected = state.candidate_spec().unwrap();

    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::Start(expected)));
}

#[test]
fn physical_candidate_starts_before_primary_graph_bindings_are_published() {
    let (mut state, mailbox) = ui(ServiceGraph::default(), true);
    state.run_limit = RunLimit::Seconds(12);
    state.max_block_frames = 512;
    let expected = state.candidate_spec().unwrap();
    let before = state.snapshot.graph.clone();
    assert!(state.can_start_transport());

    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::Start(expected)));
    assert_eq!(mailbox.try_request(), None);
    assert_eq!(state.snapshot.graph, before);
}

#[test]
fn physical_device_rows_hide_only_exact_vad_mmdevice_ids() {
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    let catalog = DeviceCatalog {
        inputs: vec![row("physical-input"), row("vad-mmdevice")],
        outputs: vec![row("physical-output"), row("vad-mmdevice")],
        vad_endpoints: [(
            VirtualEndpointId::from_bytes([0x42; 16]),
            "vad-mmdevice".into(),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    assert_eq!(
        super::input_rows(&catalog, AudioBackend::Wasapi)
            .into_iter()
            .map(|row| row.endpoint_id)
            .collect::<Vec<_>>(),
        vec!["physical-input"]
    );
    assert_eq!(
        super::output_rows(&catalog, AudioBackend::Wasapi)
            .into_iter()
            .map(|row| row.endpoint_id)
            .collect::<Vec<_>>(),
        vec!["physical-output"]
    );
}

#[test]
fn device_rows_project_vad_in_app_direction_without_reversing_physical_devices() {
    use crate::service::{ExportResourceState, ExportStatus, WindowsEndpointNameObservation};
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: "Same friendly name".into(),
    };
    let playback = VirtualEndpointId::from_bytes([0x41; 16]);
    let microphone = VirtualEndpointId::from_bytes([0x42; 16]);
    let catalog = DeviceCatalog {
        inputs: vec![row("physical-mic"), row("virtual-mic")],
        outputs: vec![row("physical-speaker"), row("virtual-speaker")],
        vad_endpoints: [
            (playback, "virtual-speaker".into()),
            (microphone, "virtual-mic".into()),
        ]
        .into_iter()
        .collect(),
        asio_drivers: vec![row("asio:driver")],
        ..Default::default()
    };
    let exports = [
        (playback, ExportDirection::Playback, "virtual-speaker"),
        (microphone, ExportDirection::Microphone, "virtual-mic"),
    ]
    .map(|(id, direction, mmdevice)| ExportStatus {
        intent: ExportIntent {
            endpoint_id: id,
            direction,
            name: "Requested".into(),
            channels: 2,
            enabled: true,
            binding: None,
        },
        windows_name: Some(WindowsEndpointNameObservation {
            native_identity: [1, 2, 3],
            observed_qpc_100ns: None,
            mmdevice_id: Some(mmdevice.into()),
            name: Some("Observed".into()),
            error: None,
        }),
        resource: ExportResourceState::Present,
        allow_default: false,
        gate_open: false,
        active_revision: None,
        #[cfg(feature = "vad-integration-probe")]
        native_stamp: None,
    });
    let ids = |rows: Vec<DeviceRow>| rows.into_iter().map(|r| r.endpoint_id).collect::<Vec<_>>();
    assert_eq!(
        ids(super::device_rows(
            &catalog,
            &exports,
            AudioBackend::Wasapi,
            true
        )),
        ["physical-mic", "virtual-speaker"]
    );
    assert_eq!(
        ids(super::device_rows(
            &catalog,
            &exports,
            AudioBackend::Wasapi,
            false
        )),
        ["physical-speaker", "virtual-mic"]
    );
    assert_eq!(
        ids(super::device_rows(
            &catalog,
            &exports,
            AudioBackend::Asio,
            true
        )),
        ["asio:driver"]
    );
    let mut retiring = exports.clone();
    retiring[0].resource = ExportResourceState::Removing;
    retiring[1].resource = ExportResourceState::Unavailable;
    assert_eq!(
        ids(super::device_rows(
            &catalog,
            &retiring,
            AudioBackend::Wasapi,
            true
        )),
        ["physical-mic"]
    );
    assert_eq!(
        ids(super::device_rows(
            &catalog,
            &retiring,
            AudioBackend::Wasapi,
            false
        )),
        ["physical-speaker"]
    );
}

#[test]
fn nonprimary_unbound_io_rejects_otherwise_valid_physical_start() {
    let mut graph = physical_graph();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Sink,
            channels: 2,
        })
        .unwrap();
    let (mut state, mailbox) = ui(graph, true);
    assert!(state.candidate_spec().is_some());
    assert!(!state.can_start_transport());

    state.toggle_transport();
    assert_eq!(mailbox.try_request(), None);
    assert!(state.dispatch_error.is_some());
}

#[test]
fn non_default_latency_profiles_get_distinct_transport_fingerprints() {
    use crate::service::AudioLatencyProfile;
    let (mut state, mailbox) = ui(physical_graph(), true);
    assert!(state.can_start_transport());

    publish_snapshot_change(&mut state, &mailbox, |snapshot| {
        snapshot.latency_profile = AudioLatencyProfile::Balanced;
    });
    let balanced = state.transport_fingerprint();
    assert!(
        state.can_start_transport(),
        "switching to Balanced must revalidate, not replay the Compatible verdict"
    );

    publish_snapshot_change(&mut state, &mailbox, |snapshot| {
        snapshot.latency_profile = AudioLatencyProfile::Low;
    });
    assert_ne!(
        state.transport_fingerprint(),
        balanced,
        "the old key folded both non-default profiles into one 'non-default' bit, \
         so a Balanced→Low switch could never invalidate the memo"
    );
    assert!(state.can_start_transport());
}

#[test]
fn the_ui_block_ceiling_is_part_of_the_transport_fingerprint() {
    let (mut state, _mailbox) = ui(physical_graph(), true);
    assert!(state.can_start_transport());
    let before = state.transport_fingerprint();
    state.max_block_frames = 512;
    assert_ne!(
        state.transport_fingerprint(),
        before,
        "the physical spec validates this ceiling, so the key must carry it"
    );
}

#[test]
fn an_unrelated_snapshot_change_keeps_the_transport_fingerprint() {
    let (mut state, mailbox) = ui(physical_graph(), true);
    assert!(state.can_start_transport());
    let before = state.transport_fingerprint();
    publish_phase(&mut state, &mailbox, SessionPhase::Running);
    assert_eq!(
        state.transport_fingerprint(),
        before,
        "unrelated snapshot fields must not invalidate the memo"
    );
    assert!(state.can_start_transport());
}

#[test]
fn catalog_refresh_without_a_graph_edit_revalidates_start_availability() {
    let (mut state, mailbox) = ui(physical_graph(), true);
    assert!(state.can_start_transport());

    // Unplug the selected output: a pure catalog refresh never bumps the
    // candidate revision, so only the availability probe can move the memo.
    publish_snapshot_change(&mut state, &mailbox, |snapshot| {
        snapshot.catalog = Arc::new(DeviceCatalog {
            inputs: full_catalog().inputs,
            ..Default::default()
        });
    });
    assert!(
        !state.can_start_transport(),
        "a vanished output row must disable start without any graph edit"
    );

    // The device comes back: the memo must not keep start disabled until
    // the next graph edit.
    publish_snapshot_change(&mut state, &mailbox, |snapshot| {
        snapshot.catalog = Arc::new(full_catalog());
    });
    assert!(
        state.can_start_transport(),
        "a restored output row must re-enable start without any graph edit"
    );
}
