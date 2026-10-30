use super::{IntentEdit, binding_candidate, create_export, edit_export};
use crate::service::{ExportDirection, ExportIntent, GraphEdit, ServiceGraph};
use moiren_core::graph::NodeKind;
use moiren_engine::node::VirtualEndpointId;

fn id(value: u8) -> VirtualEndpointId {
    VirtualEndpointId::from_bytes([value; 16])
}

#[test]
fn create_supports_both_directions_and_shapes_without_binding_or_default_authority() {
    for direction in [ExportDirection::Playback, ExportDirection::Microphone] {
        for channels in [1, 2] {
            let graph = ServiceGraph::default().snapshot();
            let exports = create_export(&graph, id(1), direction, "My endpoint", channels).unwrap();
            assert_eq!(exports.len(), 1);
            assert_eq!(exports[0].direction, direction);
            assert_eq!(exports[0].channels, channels);
            assert_eq!(exports[0].name, "My endpoint");
            assert!(exports[0].enabled);
            assert_eq!(exports[0].binding, None);
            assert!(
                graph.exports.is_empty(),
                "only a service request edits intent"
            );
        }
    }
}

#[test]
fn create_rejects_bad_names_shapes_and_duplicate_identity() {
    let mut graph = ServiceGraph::default().snapshot();
    for name in ["", "  ", "bad\nname", &"x".repeat(64), &"😀".repeat(32)] {
        assert!(create_export(&graph, id(1), ExportDirection::Playback, name, 2).is_err());
    }
    assert!(create_export(&graph, id(1), ExportDirection::Playback, "Name", 3).is_err());
    graph.exports = create_export(&graph, id(1), ExportDirection::Playback, "First", 2).unwrap();
    assert!(create_export(&graph, id(1), ExportDirection::Microphone, "Second", 1).is_err());
}

#[test]
fn edits_preserve_other_intents_identity_and_channel_shape() {
    let mut graph = ServiceGraph::default().snapshot();
    graph.exports = create_export(&graph, id(1), ExportDirection::Playback, "First", 2).unwrap();
    graph.exports = create_export(&graph, id(2), ExportDirection::Microphone, "Second", 1).unwrap();
    let other = graph.exports[1].clone();
    graph.exports = edit_export(&graph, id(1), IntentEdit::Rename("Renamed".into())).unwrap();
    assert_eq!(graph.exports[0].name, "Renamed");
    graph.exports = edit_export(&graph, id(1), IntentEdit::Enabled(false)).unwrap();
    assert!(!graph.exports[0].enabled);
    assert_eq!(graph.exports[0].endpoint_id, id(1));
    assert_eq!(graph.exports[0].direction, ExportDirection::Playback);
    assert_eq!(graph.exports[0].channels, 2);
    assert_eq!(graph.exports[1], other);
    assert!(edit_export(&graph, id(1), IntentEdit::Rename("\t".into())).is_err());
    assert!(edit_export(&graph, id(3), IntentEdit::Remove).is_err());
    assert_eq!(
        edit_export(&graph, id(1), IntentEdit::Remove).unwrap(),
        vec![other]
    );
}

#[test]
fn binding_requires_matching_direction_shape_and_exclusive_graph_io() {
    let mut service = ServiceGraph::default();
    let source = service.snapshot().graph.nodes()[0].id();
    let sink = service.snapshot().graph.nodes()[3].id();
    let gain = service.snapshot().graph.nodes()[1].id();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Source,
            channels: 1,
        })
        .unwrap();
    let mono = service.snapshot().graph.nodes()[4].id();
    service
        .edit(GraphEdit::SetExports(vec![ExportIntent {
            endpoint_id: id(1),
            direction: ExportDirection::Playback,
            name: "Playback".into(),
            channels: 2,
            enabled: true,
            binding: None,
        }]))
        .unwrap();
    assert_eq!(
        binding_candidate(&service.snapshot(), id(1), Some(source)),
        Some(source)
    );
    for selection in [None, Some(sink), Some(mono), Some(gain)] {
        assert_eq!(
            binding_candidate(&service.snapshot(), id(1), selection),
            None
        );
    }
    assert!(
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: id(1),
                node: Some(sink)
            })
            .is_err()
    );
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: id(1),
            node: Some(source),
        })
        .unwrap();
    let mut exports = service.snapshot().exports;
    exports.push(ExportIntent {
        endpoint_id: id(2),
        name: "Other".into(),
        binding: None,
        ..exports[0].clone()
    });
    service.edit(GraphEdit::SetExports(exports)).unwrap();
    assert_eq!(
        binding_candidate(&service.snapshot(), id(2), Some(source)),
        None
    );
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: id(2),
            node: Some(source),
        })
        .unwrap();
    assert_eq!(service.snapshot().exports[0].binding, None);
    assert_eq!(service.snapshot().exports[1].binding, Some(source));
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: id(2),
            node: None,
        })
        .unwrap();
    assert_eq!(
        binding_candidate(&service.snapshot(), id(1), Some(source)),
        Some(source)
    );
}

#[test]
fn create_enforces_service_export_limit() {
    let mut graph = ServiceGraph::default().snapshot();
    for value in 0..crate::service::MAX_EXPORTS as u8 {
        graph.exports =
            create_export(&graph, id(value), ExportDirection::Playback, "Export", 2).unwrap();
    }
    assert!(create_export(&graph, id(255), ExportDirection::Playback, "Overflow", 2).is_err());
}

#[test]
fn shape_edits_reject_incompatible_binding_then_allow_unbind_and_rebind() {
    for direction in [ExportDirection::Playback, ExportDirection::Microphone] {
        let mut service = ServiceGraph::default();
        let stereo = if direction == ExportDirection::Playback {
            service.snapshot().graph.nodes()[0].id()
        } else {
            service.snapshot().graph.nodes()[3].id()
        };
        service
            .edit(GraphEdit::AddNode {
                kind: direction.node_kind(),
                channels: 1,
            })
            .unwrap();
        let mono = service.snapshot().graph.nodes()[4].id();
        service
            .edit(GraphEdit::SetExports(vec![ExportIntent {
                endpoint_id: id(1),
                direction,
                name: "Endpoint".into(),
                channels: 2,
                enabled: true,
                binding: None,
            }]))
            .unwrap();
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: id(1),
                node: Some(stereo),
            })
            .unwrap();
        let before = service.snapshot();
        assert!(edit_export(&before, id(1), IntentEdit::Channels(1)).is_err());
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: id(1),
                node: None,
            })
            .unwrap();
        let mut candidate = service.snapshot();
        candidate.exports = edit_export(&candidate, id(1), IntentEdit::Channels(1)).unwrap();
        service
            .edit(GraphEdit::SetExports(candidate.exports.clone()))
            .unwrap();
        let exports = candidate.exports;
        assert_eq!(exports[0].endpoint_id, id(1));
        assert_eq!(exports[0].name, "Endpoint");
        assert_eq!(exports[0].direction, direction);
        assert!(exports[0].enabled);
        assert_eq!(
            binding_candidate(&service.snapshot(), id(1), Some(stereo)),
            None
        );
        assert_eq!(
            binding_candidate(&service.snapshot(), id(1), Some(mono)),
            Some(mono)
        );
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: id(1),
                node: Some(mono),
            })
            .unwrap();
        assert!(edit_export(&service.snapshot(), id(1), IntentEdit::Channels(2)).is_err());
        assert!(edit_export(&service.snapshot(), id(1), IntentEdit::Channels(0)).is_err());
        assert!(edit_export(&service.snapshot(), id(1), IntentEdit::Channels(3)).is_err());
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: id(1),
                node: None,
            })
            .unwrap();
        assert_eq!(
            binding_candidate(&service.snapshot(), id(1), Some(stereo)),
            None
        );
        assert_eq!(
            binding_candidate(&service.snapshot(), id(1), Some(mono)),
            Some(mono)
        );
    }
}

#[cfg(feature = "gpui-test")]
mod headless {
    use super::super::super::super::{StudioShell, UiState, devices, pages};
    use crate::service::{AppSnapshot, ExportDirection, GraphSnapshot, ServiceRuntime};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Entity, ScrollDelta, SharedString, VisualTestContext, point, px, size,
    };

    fn wait_for(
        view: &Entity<StudioShell>,
        cx: &mut VisualTestContext,
        ready: impl Fn(&AppSnapshot) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let done = view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                let state = shell.state.read(cx);
                !state.pending_request && ready(&state.snapshot)
            });
            if done {
                cx.update(|window, cx| window.render_frame(cx));
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "export request timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn candidate(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> GraphSnapshot {
        view.update(cx, |shell, cx| {
            (*shell.state.read(cx).snapshot.graph).clone()
        })
    }

    fn scroll_to_control(cx: &mut VisualTestContext, id: impl Into<SharedString>) {
        let id = id.into();
        for _ in 0..80 {
            let ready = cx.update(|window, cx| {
                window.render_frame(cx);
                let page = window.find("devices-page").bounds();
                let top = page.top() + px(64.);
                let bottom = page.bottom() - px(16.);
                let target = window.try_find(id.clone());
                if let Some(target) = &target {
                    let bounds = target.bounds();
                    if target.visible() && bounds.top() >= top && bounds.bottom() <= bottom {
                        assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
                        assert!(bounds.left() >= page.left() + px(16.), "{id}: {bounds:?}");
                        assert!(bounds.right() <= page.right() - px(16.), "{id}: {bounds:?}");
                        return true;
                    }
                }
                let delta = if target.is_some_and(|target| target.bounds().top() < top) {
                    100.
                } else {
                    -100.
                };
                window.scroll(
                    "devices-page",
                    ScrollDelta::Pixels(point(px(0.), px(delta))),
                    cx,
                );
                false
            });
            if ready {
                return;
            }
        }
        panic!("control {id} cannot be fully reached by scrolling at 1024x720");
    }

    #[gpui_kit::test]
    fn compact_exports_controls_remain_reachable_by_scrolling_at_1024x720(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
        let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
        let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
        view.update(cx, |shell, cx| {
            shell.preferences_path = None;
            shell.preferences.export_authorizations.clear();
            shell.presentation.page = pages::Page::Devices;
            shell.device_tab = devices::DeviceTab::Exports;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.resize(size(px(1024.), px(720.)));
            window.render_frame(cx);
        });
        for id in [
            "export-create-playback",
            "export-create-microphone",
            "export-create-mono",
            "export-create-stereo",
            "export-create-name",
            "export-create",
        ] {
            scroll_to_control(cx, id);
        }
        scroll_to_control(cx, "export-create");
        cx.update(|window, cx| window.click("export-create", cx));
        wait_for(&view, cx, |snapshot| snapshot.graph.exports.len() == 1);
        for count in 2..=3 {
            scroll_to_control(cx, "export-create");
            cx.update(|window, cx| window.click("export-create", cx));
            wait_for(&view, cx, |snapshot| snapshot.graph.exports.len() == count);
        }
        let exports = candidate(&view, cx).exports;
        // The service projects rows in endpoint-ID order, not creation order.
        let id = exports
            .iter()
            .map(|intent| intent.endpoint_id)
            .max()
            .unwrap();
        let key = |action| SharedString::from(format!("export-{action}-{id}"));
        for action in [
            "mono", "stereo", "name", "rename", "toggle", "bind", "unbind", "remove", "default",
        ] {
            scroll_to_control(cx, key(action));
        }
        scroll_to_control(cx, key("default"));
        cx.update(|window, cx| window.click(key("default"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot
                .exports
                .iter()
                .any(|status| status.intent.endpoint_id == id && status.allow_default)
        });
        scroll_to_control(cx, key("mono"));
        cx.update(|window, cx| window.click(key("mono"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot
                .graph
                .exports
                .iter()
                .any(|intent| intent.endpoint_id == id && intent.channels == 1)
        });
        scroll_to_control(cx, key("toggle"));
        cx.update(|window, cx| window.click(key("toggle"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot
                .graph
                .exports
                .iter()
                .any(|intent| intent.endpoint_id == id && !intent.enabled)
        });
        scroll_to_control(cx, "export-create-name");
        cx.update(|window, cx| {
            window.click("export-create-name", cx);
            window.input("Compact", cx);
        });
        view.update(cx, |shell, cx| {
            assert!(
                shell
                    .export_controls
                    .name
                    .read(cx)
                    .value()
                    .contains("Compact")
            )
        });
        scroll_to_control(cx, "export-create");
        assert_eq!(candidate(&view, cx).exports.len(), 3);
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[gpui_kit::test]
    fn export_controls_create_rename_bind_unbind_toggle_and_remove(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
        let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
        let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
        view.update(cx, |shell, cx| {
            shell.preferences_path = None;
            shell.preferences.export_authorizations.clear();
            shell.presentation.page = pages::Page::Devices;
            shell.device_tab = devices::DeviceTab::Exports;
            shell.inspector_open = false;
            shell.sidebar_open = false;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.resize(size(px(1440.), px(1600.)));
            window.render_frame(cx);
            for control in [
                "export-create-playback",
                "export-create-microphone",
                "export-create-mono",
                "export-create-stereo",
                "export-create-name",
                "export-create",
            ] {
                assert!(window.find(control).visible(), "missing control {control}");
            }
            let help = window.find("export-default-help");
            let help = help.label().unwrap();
            assert!(
                help.contains(
                    "Off prevents both Windows automatic and user manual default selection"
                )
            );
            assert!(
                help.contains("On only permits eligibility; it does not set a Windows default")
            );
            assert!(
                help.contains("this computer only") && help.contains("not saved in the project")
            );
            window.click("export-create", cx);
        });
        wait_for(&view, cx, |snapshot| snapshot.graph.exports.len() == 1);
        let first = candidate(&view, cx).exports[0].clone();
        assert_eq!(first.direction, ExportDirection::Playback);
        assert_eq!(first.channels, 2);
        assert!(first.enabled && first.binding.is_none());
        assert_eq!(candidate(&view, cx).active_revision, None);
        let id = first.endpoint_id;
        let key = |action| SharedString::from(format!("export-{action}-{id}"));
        let graph = candidate(&view, cx);
        let source = graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Source)
            .unwrap()
            .id();
        let sink = graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Sink)
            .unwrap()
            .id();
        view.update(cx, |shell, cx| {
            shell.selected_node = Some(sink);
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click(key("bind"), cx);
            window.click(key("unbind"), cx);
        });
        view.update(cx, |shell, cx| {
            assert!(
                !shell.state.read(cx).pending_request,
                "incompatible controls are inert"
            );
        });
        assert_eq!(candidate(&view, cx).exports[0].binding, None);
        view.update(cx, |shell, cx| {
            shell.selected_node = Some(source);
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click(key("bind"), cx);
        });
        wait_for(&view, cx, |snapshot| {
            snapshot.graph.exports[0].binding == Some(source)
        });
        assert_eq!(
            candidate(&view, cx).active_revision,
            None,
            "binding edits do not activate audio"
        );
        cx.update(|window, cx| {
            window.click(key("mono"), cx);
            window.render_frame(cx);
            assert!(window.find("error-strip").visible());
        });
        assert_eq!(candidate(&view, cx).exports[0].channels, 2);
        assert_eq!(candidate(&view, cx).exports[0].binding, Some(source));
        cx.update(|window, cx| window.click(key("unbind"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot.graph.exports[0].binding.is_none()
        });
        cx.update(|window, cx| window.click(key("mono"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot.graph.exports[0].channels == 1
        });
        cx.update(|window, cx| window.click(key("bind"), cx));
        assert_eq!(
            candidate(&view, cx).exports[0].binding,
            None,
            "stereo selection cannot bind mono export"
        );
        cx.update(|window, cx| window.click(key("stereo"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot.graph.exports[0].channels == 2
        });
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.export_controls.names[&id]
                    .input
                    .update(cx, |input, cx| {
                        input.set_value("Renamed Playback", window, cx)
                    });
            });
            window.render_frame(cx);
            window.click(key("rename"), cx);
        });
        wait_for(&view, cx, |snapshot| {
            snapshot.graph.exports[0].name == "Renamed Playback"
        });
        cx.update(|window, cx| window.click(key("toggle"), cx));
        wait_for(&view, cx, |snapshot| !snapshot.graph.exports[0].enabled);
        cx.update(|window, cx| window.click(key("toggle"), cx));
        wait_for(&view, cx, |snapshot| snapshot.graph.exports[0].enabled);
        cx.update(|window, cx| window.click(key("default"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot
                .exports
                .iter()
                .any(|status| status.intent.endpoint_id == id && status.allow_default)
        });
        view.update(cx, |shell, _| {
            assert_eq!(shell.preferences.export_authorizations.len(), 1);
            assert!(shell.preferences.export_authorizations[0].allow_default);
        });
        cx.update(|window, cx| window.click(key("default"), cx));
        wait_for(&view, cx, |snapshot| {
            snapshot
                .exports
                .iter()
                .any(|status| status.intent.endpoint_id == id && !status.allow_default)
        });
        cx.update(|window, cx| window.click(key("remove"), cx));
        wait_for(&view, cx, |snapshot| snapshot.graph.exports.is_empty());
        cx.update(|window, cx| {
            window.click("export-create-microphone", cx);
            window.render_frame(cx);
            window.click("export-create-mono", cx);
            window.render_frame(cx);
            view.update(cx, |shell, cx| {
                shell
                    .export_controls
                    .name
                    .update(cx, |input, cx| input.set_value("My Microphone", window, cx));
            });
            window.render_frame(cx);
            window.click("export-create", cx);
        });
        wait_for(&view, cx, |snapshot| snapshot.graph.exports.len() == 1);
        let microphone = candidate(&view, cx).exports[0].clone();
        assert_ne!(
            microphone.endpoint_id, id,
            "removed identities must not be reused"
        );
        assert_eq!(microphone.direction, ExportDirection::Microphone);
        assert_eq!(microphone.channels, 1);
        assert_eq!(microphone.name, "My Microphone");
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[gpui_kit::test]
    fn invalid_create_and_rename_show_existing_feedback_without_losing_drafts(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
        let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
        let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
        view.update(cx, |shell, cx| {
            shell.preferences_path = None;
            shell.presentation.page = pages::Page::Devices;
            shell.device_tab = devices::DeviceTab::Exports;
            shell.inspector_open = false;
            shell.sidebar_open = false;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.resize(size(px(1440.), px(1600.)));
            view.update(cx, |shell, cx| {
                shell
                    .export_controls
                    .name
                    .update(cx, |input, cx| input.set_value("  ", window, cx))
            });
            window.render_frame(cx);
            window.click("export-create", cx);
            window.render_frame(cx);
            assert!(window.find("error-strip").visible());
        });
        assert!(candidate(&view, cx).exports.is_empty());
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                assert!(
                    shell
                        .state
                        .read(cx)
                        .dispatch_error
                        .as_ref()
                        .unwrap()
                        .contains("UTF-16")
                );
                shell
                    .export_controls
                    .name
                    .update(cx, |input, cx| input.set_value("Valid", window, cx));
            });
            window.render_frame(cx);
            window.click("export-create", cx);
        });
        wait_for(&view, cx, |snapshot| snapshot.graph.exports.len() == 1);
        let id = candidate(&view, cx).exports[0].endpoint_id;
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.export_controls.names[&id]
                    .input
                    .update(cx, |input, cx| input.set_value("\t", window, cx))
            });
            window.render_frame(cx);
            window.click(SharedString::from(format!("export-rename-{id}")), cx);
            window.render_frame(cx);
            assert!(window.find("error-strip").visible());
        });
        view.update(cx, |shell, cx| {
            assert_eq!(
                shell.export_controls.names[&id]
                    .input
                    .read(cx)
                    .value()
                    .as_ref(),
                "\t"
            );
        });
        assert_eq!(candidate(&view, cx).exports[0].name, "Valid");
        handle.request_exit();
        runtime.join().unwrap();
    }
}
