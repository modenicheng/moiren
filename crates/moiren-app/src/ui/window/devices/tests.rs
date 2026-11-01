use super::export_bind_candidate;
use crate::service::{ExportDirection, ExportIntent, GraphSnapshot};
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::node::VirtualEndpointId;

#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn application_selector_explains_tree_scope_without_starting_or_substituting_a_process(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{
        AppRequest, DeviceCatalog, DeviceRow, GraphEdit, InputSelection, ProcessRow,
        ServiceContext, ServiceMailbox,
    };
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, px, size};
    use std::sync::Arc;

    cx.update(gpui_kit::init);
    let mut context = ServiceContext::default();
    context.catalog = Arc::new(DeviceCatalog {
        outputs: vec![DeviceRow {
            endpoint_id: "scope-fixture-output".into(),
            name: "Fixture output".into(),
        }],
        ..Default::default()
    });
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    let original_graph = initial.graph.clone();
    let state = cx.new(|_| super::super::UiState::new(handle, initial));
    let (view, cx) = super::super::test_harness::shell_with_state(cx, state);
    while mailbox.try_request().is_some() {}
    view.update(cx, |shell, cx| {
        shell.presentation.page = super::super::pages::Page::Devices;
        shell.device_tab = super::DeviceTab::Applications;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.resize(size(px(1440.), px(900.)));
        window.render_frame(cx);
        let scope = window
            .try_find("application-capture-scope")
            .expect("even the empty Applications list must explain its capture scope");
        assert!(scope.visible());
        assert_eq!(
            scope.label(),
            Some(rust_i18n::t!(
                "Captures the selected process and its descendants, not an individual browser tab or audio session."
            ).as_ref())
        );
    });
    assert!(mailbox.try_request().is_none());

    let mut catalog = (*context.catalog).clone();
    catalog.processes = vec![
        ProcessRow {
            pid: 42,
            creation_time_100ns: 123,
            name: "Same player name".into(),
        },
        ProcessRow {
            pid: 43,
            creation_time_100ns: 124,
            name: "Same player name".into(),
        },
    ];
    context.catalog = Arc::new(catalog);
    mailbox.publish(crate::service::snapshot(&context));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("application-capture-scope").visible());
        assert!(window.find("process-42-123").visible());
        window.click("process-42-123", cx);
    });
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert_eq!(state.selection.process_key, Some((42, 123)));
        assert_eq!(state.snapshot.graph, original_graph);
    });
    let mut bindings = 0;
    while let Some(request) = mailbox.try_request() {
        match request {
            AppRequest::EditGraph(GraphEdit::BindSource { source, .. }) => {
                assert_eq!(
                    source,
                    InputSelection::Process {
                        pid: 42,
                        creation_time_100ns: 123
                    }
                );
                bindings += 1;
            }
            AppRequest::EditGraph(GraphEdit::BindOutput { endpoint_id, .. }) => {
                assert_eq!(endpoint_id, "scope-fixture-output");
            }
            other => panic!("selection must not start/apply audio: {other:?}"),
        }
    }
    assert_eq!(bindings, 1);
}

#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn owned_vad_rows_bind_in_opposite_app_directions_without_starting_audio(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{
        AppRequest, DeviceCatalog, DeviceRow, ExportObservation, ExportResourceState, GraphEdit,
        RequestId, ServiceContext, ServiceMailbox, WindowsEndpointNameObservation,
    };
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, SharedString};
    use std::sync::Arc;
    cx.update(gpui_kit::init);
    let mut context = ServiceContext::default();
    let endpoints = [
        (
            VirtualEndpointId::from_bytes([0x41; 16]),
            ExportDirection::Playback,
            "virtual-speaker",
        ),
        (
            VirtualEndpointId::from_bytes([0x42; 16]),
            ExportDirection::Microphone,
            "virtual-microphone",
        ),
    ];
    let intents = endpoints.map(|(endpoint_id, direction, name)| ExportIntent {
        endpoint_id,
        direction,
        name: name.into(),
        channels: 2,
        enabled: true,
        binding: None,
    });
    context
        .graph
        .edit(GraphEdit::SetExports(intents.to_vec()))
        .unwrap();
    context.exports.set_available(true);
    for intent in intents {
        context
            .exports
            .observe(ExportObservation {
                intent: intent.clone(),
                generation: 1,
                resource: ExportResourceState::Present,
                allow_default: false,
                gate_open: false,
                active_revision: None,
            })
            .unwrap();
    }
    let project_snapshot = |context: &ServiceContext| {
        let mut snapshot = crate::service::snapshot(context);
        for status in &mut snapshot.exports {
            status.windows_name = Some(WindowsEndpointNameObservation {
                native_identity: [1, 2, 3],
                observed_qpc_100ns: None,
                mmdevice_id: Some(status.intent.name.clone()),
                name: Some(status.intent.name.clone()),
                error: None,
            });
        }
        snapshot
    };
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    context.catalog = Arc::new(DeviceCatalog {
        inputs: vec![row("physical-microphone"), row("virtual-microphone")],
        outputs: vec![row("physical-speaker"), row("virtual-speaker")],
        vad_endpoints: endpoints
            .map(|(id, _, name)| (id, name.to_string()))
            .into_iter()
            .collect(),
        ..Default::default()
    });
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), project_snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    let state = cx.new(|_| super::super::UiState::new(handle, initial));
    let (view, cx) =
        cx.add_window_view(|window, cx| super::super::StudioShell::new(state, window, cx, None));
    while mailbox.try_request().is_some() {}
    for (input, (endpoint_id, direction, name)) in [true, false].into_iter().zip(endpoints) {
        view.update(cx, |shell, cx| {
            shell.preferences_path = None;
            shell.presentation.page = super::super::pages::Page::Devices;
            shell.device_tab = if input {
                super::DeviceTab::Inputs
            } else {
                super::DeviceTab::Outputs
            };
            shell
                .state
                .update(cx, |state, _| state.pending_request = false);
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find(SharedString::from(format!("device-row-{name}")))
                    .visible()
            );
            let wrong = if input {
                "virtual-microphone"
            } else {
                "virtual-speaker"
            };
            assert!(
                window
                    .try_find(SharedString::from(format!("device-row-{wrong}")))
                    .is_none()
            );
            window.click(SharedString::from(format!("choose-{name}")), cx);
        });
        let request = mailbox
            .try_request()
            .expect("Bind click dispatches an edit");
        let AppRequest::EditGraph(GraphEdit::BindExport {
            endpoint_id: actual,
            node: Some(node),
        }) = request
        else {
            panic!("must dispatch BindExport, not physical binding or audio start");
        };
        assert_eq!(actual, endpoint_id);
        assert_eq!(
            context
                .graph
                .snapshot()
                .graph
                .get_node(node)
                .unwrap()
                .kind(),
            direction.node_kind()
        );
        context
            .graph
            .edit(GraphEdit::BindExport {
                endpoint_id,
                node: Some(node),
            })
            .unwrap();
        let mut snapshot = project_snapshot(&context);
        snapshot.consumed_request_id = RequestId(100);
        mailbox.publish(snapshot);
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            cx.notify();
        });
    }
    assert!(
        context
            .exports
            .preflight_audio(&context.graph.snapshot().exports)
            .is_err()
    );
    assert!(
        mailbox.try_request().is_none(),
        "no Start or ApplyGraph request"
    );
}

fn snapshot(graph: LogicalGraph, exports: Vec<ExportIntent>) -> GraphSnapshot {
    GraphSnapshot {
        exports,
        graph,
        nodes: Vec::new(),
        candidate_revision: 0,
        active_revision: None,
        pending_revision: None,
        last_error: None,
        faults: Vec::new(),
        can_undo: false,
        can_redo: false,
    }
}

fn export(binding: Option<moiren_core::graph::NodeId>) -> ExportIntent {
    ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([0x7e; 16]),
        direction: ExportDirection::Playback,
        name: "export".into(),
        channels: 2,
        enabled: true,
        binding,
    }
}

fn stereo_export_bound_to(node: moiren_core::graph::NodeId) -> ExportIntent {
    ExportIntent {
        endpoint_id: VirtualEndpointId::from_bytes([0x7d; 16]),
        direction: ExportDirection::Playback,
        name: "other".into(),
        channels: 2,
        enabled: true,
        binding: Some(node),
    }
}

#[test]
fn quick_bind_skips_channel_mismatches() {
    let mut graph = LogicalGraph::new();
    let mono = graph.create_node(NodeKind::Source, 1).unwrap();
    let stereo = graph.create_node(NodeKind::Source, 2).unwrap();
    assert_eq!(
        export_bind_candidate(
            &snapshot(graph, vec![export(None)]),
            ExportDirection::Playback,
            2
        ),
        Some(stereo)
    );
    assert_eq!(
        export_bind_candidate(
            &snapshot(LogicalGraph::new(), vec![]),
            ExportDirection::Playback,
            2
        )
        .map(|_| ()),
        None
    );
    let _ = mono;
}

#[test]
fn quick_bind_prefers_an_unoccupied_match_and_falls_back_to_steal() {
    let mut graph = LogicalGraph::new();
    let first = graph.create_node(NodeKind::Source, 2).unwrap();
    let second = graph.create_node(NodeKind::Source, 2).unwrap();
    // Another export holds the first match; the shortcut must pick the
    // free second node instead of silently rebinding the occupied one.
    assert_eq!(
        export_bind_candidate(
            &snapshot(
                graph.clone(),
                vec![export(None), stereo_export_bound_to(first)]
            ),
            ExportDirection::Playback,
            2
        ),
        Some(second)
    );
    // With every match occupied, the previous behavior of binding the
    // first match (the service edit steals it) is preserved.
    let mut single = LogicalGraph::new();
    let only = single.create_node(NodeKind::Source, 2).unwrap();
    assert_eq!(
        export_bind_candidate(
            &snapshot(single, vec![export(None), stereo_export_bound_to(only)]),
            ExportDirection::Playback,
            2
        ),
        Some(only)
    );
}
