use super::export_bind_candidate;
use crate::service::{ExportDirection, ExportIntent, GraphSnapshot};
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::node::VirtualEndpointId;

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

#[cfg(feature = "gpui-test")]
use gpui_kit::{Entity, VisualTestContext};

/// One cached-path frame: notified views re-render, unnotified cached
/// subtrees replay their paint. `render_frame` refreshes the whole window and
/// would rebuild every panel regardless of the caches.
#[cfg(feature = "gpui-test")]
fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

#[cfg(feature = "gpui-test")]
fn devices_rebuilds(view: &Entity<super::super::StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| shell.devices_panel.read(cx).rebuild_count())
}

/// Delivers a snapshot the way the service does — a fresh Arc behind a shell
/// notification — without a full-window `render_frame`, which rebuilds every
/// panel regardless of the caches.
#[cfg(feature = "gpui-test")]
fn publish_snapshot(
    view: &Entity<super::super::StudioShell>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut crate::service::AppSnapshot),
) {
    view.update(cx, |shell, cx| {
        let mut snapshot = (*shell.state.read(cx).snapshot).clone();
        change(&mut snapshot);
        shell.state.update(cx, |state, _| {
            state.snapshot = std::sync::Arc::new(snapshot)
        });
        cx.notify();
    });
    cached_frame(cx);
}

/// The devices page never renders the graph selection: a selection edit plus
/// the shell notifications it emits must leave the panel on its cached paint,
/// and the replayed paint must hold across further key-neutral notifications.
#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn devices_panel_stays_cached_across_key_neutral_shell_notifications(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::size;
    use gpui_kit::test::TestWindowExt;
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    view.update(cx, |shell, cx| {
        shell.set_page(super::super::pages::Page::Devices, cx);
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    let page = cx.update(|window, _| window.find("devices-page").bounds());
    let before = devices_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        devices_rebuilds(&view, cx),
        before,
        "a selection edit must not reach the devices panel"
    );
    assert_eq!(
        cx.update(|window, _| window.find("devices-page").bounds()),
        page,
        "the cached paint must not move"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        devices_rebuilds(&view, cx),
        before,
        "a key-neutral notification must replay the cached paint"
    );
    finish_service(runtime);
}

/// The devices key carries publish identity, so a catalog publish must paint
/// its new rows on that notification alone and hold them across follow-up
/// notifications that carry no publish.
#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn devices_panel_repaints_a_published_catalog_row_exactly_once(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::DeviceRow;
    use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size};
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.set_page(super::super::pages::Page::Devices, cx);
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    assert!(
        cx.update(|window, _| window.try_find("device-row-probe-speakers").is_none()),
        "the probe row must not exist before the publish"
    );
    let before = devices_rebuilds(&view, cx);

    publish_snapshot(&view, cx, |snapshot| {
        use moiren_engine::audio_config::AudioBackend;
        snapshot.audio_settings.backend = AudioBackend::Wasapi;
        std::sync::Arc::make_mut(&mut snapshot.catalog)
            .outputs
            .push(DeviceRow {
                endpoint_id: "probe-speakers".into(),
                name: "Probe speakers".into(),
            });
    });
    assert_eq!(
        devices_rebuilds(&view, cx),
        before + 1,
        "the catalog publish must rebuild the panel exactly once"
    );
    let row = cx.update(|window, _| window.find("device-row-probe-speakers").bounds());
    assert!(
        row.size.width > px(0.),
        "the published row must be laid out"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        devices_rebuilds(&view, cx),
        before + 1,
        "a notification without a publish must replay the cached rows"
    );
    assert_eq!(
        cx.update(|window, _| window.find("device-row-probe-speakers").bounds()),
        row,
        "the replayed paint must not move the published row"
    );
    finish_service(runtime);
}
