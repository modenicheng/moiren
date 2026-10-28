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

fn ui(graph: ServiceGraph, physical_available: bool) -> (UiState, ServiceMailbox) {
    let mut context = crate::service::ServiceContext::default();
    context.graph = graph;
    if physical_available {
        let row = |id: &str| DeviceRow {
            endpoint_id: id.into(),
            name: id.into(),
        };
        context.catalog = Arc::new(DeviceCatalog {
            inputs: vec![row("input")],
            outputs: vec![row("output")],
            default_input_endpoint_id: Some("input".into()),
            default_output_endpoint_id: Some("output".into()),
            ..Default::default()
        });
    }
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    (UiState::new(handle, initial), mailbox)
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
fn physical_row_projection_hides_only_exact_vad_mmdevice_id_matches() {
    let row = |id: &str| crate::service::DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    let catalog = DeviceCatalog {
        inputs: vec![row("physical-input"), row("VAD endpoint")],
        outputs: vec![row("physical-output"), row("VAD endpoint")],
        vad_endpoint_ids: vec!["VAD endpoint".into()],
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
