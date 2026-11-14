/// Design 23 R1, held by a source scan instead of convention: the devices
/// page reads the graph and never sends a binding edit. Any of the four
/// binding-edit identifiers appearing in the page's production sources
/// fails here — binding lives on the node (canvas chip, node inspector).
/// Test modules are exempt: they legitimately build bound graphs.
#[test]
fn devices_page_sources_never_name_a_binding_edit() {
    let pages = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/pages");
    let mut files = vec![pages.join("devices.rs")];
    let mut stack = vec![pages.join("devices")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("devices page directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                files.push(path);
            }
        }
    }
    assert!(
        files.len() >= 4,
        "the scan must see the page and its submodules, found {files:?}"
    );
    let mut hits = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("devices page source");
        for needle in ["BindSource", "BindOutput", "BindExport", "ClearBinding"] {
            for (index, line) in source.lines().enumerate() {
                if line.contains(needle) {
                    hits.push(format!("{}:{}: {needle}", file.display(), index + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the devices page must not send binding edits (design 23 R1); bind on the node instead:\n{}",
        hits.join("\n")
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
    view.read_with(cx, |shell, cx| shell.devices.panel.read(cx).rebuild_count())
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
    use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::size;
    use gpui_kit::test::TestWindowExt;
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    view.update(cx, |shell, cx| {
        shell.set_page(crate::ui::pages::Page::Devices, cx);
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    let page = cx.update(|window, _| window.find("devices-page").bounds());
    let before = devices_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.selection.node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
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
    use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size};
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.set_page(crate::ui::pages::Page::Devices, cx);
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

/// The devices page and the inspector render from the settings editor's
/// device draft, keyed by its edit revision: the Override edit mutates only
/// that draft, so the one notification it emits must rebuild both cached
/// panels — the row badge on the Devices page, the stream editor in the
/// inspector — and follow-up key-neutral notifications must replay the
/// fresh paint.
#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn override_draft_revision_rebuilds_the_devices_and_inspector_panels(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::DeviceRow;
    use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::size;
    use gpui_kit::test::TestWindowExt;
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    view.update(cx, |shell, cx| {
        shell.set_page(crate::ui::pages::Page::Devices, cx);
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    // The core-driver service ships an empty catalog; publish one output
    // row to configure, exactly as a device refresh would deliver it.
    publish_snapshot(&view, cx, |snapshot| {
        std::sync::Arc::make_mut(&mut snapshot.catalog)
            .outputs
            .push(DeviceRow {
                endpoint_id: "probe-speakers".into(),
                name: "Probe speakers".into(),
            });
    });
    view.update(cx, |shell, cx| {
        shell.inspect_device("probe-speakers".into(), cx);
    });
    cached_frame(cx);
    assert!(
        cx.update(|window, _| window.find("device-override").visible()),
        "the override editor must be showing"
    );
    // Absorb the first-notification rebuild the retained-frame bookkeeping
    // can spend after a full-window refresh, then read both baselines.
    for _ in 0..3 {
        let devices_stable = devices_rebuilds(&view, cx);
        let inspector_stable = view.read_with(cx, |shell, cx| {
            shell.inspector.panel.read(cx).rebuild_count()
        });
        view.update(cx, |_, cx| cx.notify());
        cached_frame(cx);
        if devices_rebuilds(&view, cx) == devices_stable
            && view.read_with(cx, |shell, cx| {
                shell.inspector.panel.read(cx).rebuild_count()
            }) == inspector_stable
        {
            break;
        }
    }
    let devices_before = devices_rebuilds(&view, cx);
    let inspector_before = view.read_with(cx, |shell, cx| {
        shell.inspector.panel.read(cx).rebuild_count()
    });

    // Drive the Override button's handler body directly: `window.click`
    // runs a full-window `render_frame` internally, which rebuilds every
    // panel regardless of the caches and would drown the exact count. The
    // shell notification this handler emits is the cache-path delivery.
    view.update(cx, |shell, cx| {
        crate::ui::shell::settings::device_stream_mut(
            &mut shell.settings.editor.device.audio,
            crate::ui::shell::settings::Direction::Output,
            "probe-speakers",
        );
        shell.settings.editor.device.changed();
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        devices_rebuilds(&view, cx),
        devices_before + 1,
        "the draft edit must rebuild the devices page exactly once"
    );
    assert_eq!(
        view.read_with(cx, |shell, cx| shell
            .inspector
            .panel
            .read(cx)
            .rebuild_count()),
        inspector_before + 1,
        "the draft edit must rebuild the inspector exactly once"
    );
    // Paint witnesses: the override materialized in the draft and the
    // per-device stream editor replaced the defaults note.
    assert!(
        view.read_with(cx, |shell, _| shell
            .settings
            .editor
            .device
            .audio
            .output_overrides
            .contains_key("probe-speakers")),
        "the Override edit must write the draft"
    );
    assert!(
        cx.update(|window, _| window.find("device-output-rate-trigger").visible()),
        "the rebuilt inspector must show the device stream editor"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    let devices_after_edit = devices_rebuilds(&view, cx);
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        devices_rebuilds(&view, cx),
        devices_after_edit,
        "a notification without a draft edit must replay the cached rows"
    );
    let inspector_after_edit = view.read_with(cx, |shell, cx| {
        shell.inspector.panel.read(cx).rebuild_count()
    });
    assert!(
        inspector_after_edit > inspector_before,
        "the draft edit must reach the cached inspector"
    );
    finish_service(runtime);
}

/// Design 23 §3: the four tabs carry no routing or run-state controls. A
/// device row's only interaction opens its detail view; the detail lists the
/// candidate graph's users read-only; "Locate in graph" is navigation — it
/// switches to the Graph page, selects the node and opens the inspector
/// without sending any request. Moiren's own virtual endpoints never appear
/// on the physical tabs.
#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn device_rows_open_the_detail_view_and_locate_users_without_routing(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::DeviceRow;
    use crate::ui::pages::Page;
    use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{SharedString, px, size};
    use moiren_core::graph::NodeKind;
    use moiren_engine::node::VirtualEndpointId;
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.preferences.path = None;
        shell.set_page(Page::Devices, cx);
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    let sink = view.read_with(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Sink)
            .expect("the default graph has a sink")
            .id()
    });
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    publish_snapshot(&view, cx, |snapshot| {
        use moiren_engine::audio_config::AudioBackend;
        snapshot.audio_settings.backend = AudioBackend::Wasapi;
        let catalog = std::sync::Arc::make_mut(&mut snapshot.catalog);
        catalog.inputs = vec![row("probe-microphone"), row("virtual-microphone-mm")];
        catalog.outputs = vec![row("probe-speakers"), row("virtual-speaker-mm")];
        catalog.vad_endpoints = [
            (
                VirtualEndpointId::from_bytes([0x41; 16]),
                "virtual-speaker-mm".to_string(),
            ),
            (
                VirtualEndpointId::from_bytes([0x42; 16]),
                "virtual-microphone-mm".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        std::sync::Arc::make_mut(&mut snapshot.graph)
            .nodes
            .iter_mut()
            .find(|entry| entry.id == sink)
            .expect("sink state")
            .output_endpoint_id = Some("probe-speakers".into());
    });

    // No tab offers a routing or run-state control, and the applications
    // tab is gone: applications are not devices.
    cx.update(|window, _| {
        assert!(window.try_find("devices-apps").is_none());
        for tab in [
            "devices-inputs",
            "devices-outputs",
            "devices-exports",
            "devices-clock",
        ] {
            assert!(window.find(tab).visible(), "missing tab {tab}");
        }
        assert!(window.find("device-row-probe-speakers").visible());
        for removed in ["choose-probe-speakers", "inspect-probe-speakers"] {
            assert!(
                window.try_find(removed).is_none(),
                "{removed} must not exist"
            );
        }
        assert!(
            window.try_find("device-row-virtual-speaker-mm").is_none(),
            "Moiren's own virtual endpoint must not list as a physical output"
        );
    });
    view.update(cx, |shell, cx| {
        shell.devices.device_tab = super::DeviceTab::Inputs;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("device-row-probe-microphone").visible());
        assert!(window.try_find("choose-probe-microphone").is_none());
        assert!(
            window
                .try_find("device-row-virtual-microphone-mm")
                .is_none(),
            "Moiren's own virtual endpoint must not list as a physical input"
        );
    });

    // The row is the interaction: clicking it opens the device detail.
    view.update(cx, |shell, cx| {
        shell.devices.device_tab = super::DeviceTab::Outputs;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("device-row-probe-speakers", cx);
    });
    view.read_with(cx, |shell, _| {
        assert_eq!(
            shell.devices.selected_device.as_deref(),
            Some("probe-speakers")
        );
        assert!(shell.chrome.inspector_open);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("device-override").visible());
        assert!(
            window
                .find(SharedString::from(format!("device-user-{sink:?}")))
                .visible(),
            "the bound sink must list as this device's user"
        );
    });
    let before = view.read_with(cx, |shell, cx| {
        (
            shell.state.read(cx).snapshot.graph.candidate_revision,
            shell.state.read(cx).pending_request,
        )
    });

    // Locate is navigation only: page, selection and inspector change; the
    // graph and the request state do not.
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("reveal-{sink:?}")), cx);
    });
    view.read_with(cx, |shell, cx| {
        assert_eq!(shell.presentation.page(), Page::Graph);
        assert_eq!(shell.selection.node, Some(sink));
        assert!(shell.selection.nodes.contains(&sink));
        assert_eq!(shell.selection.nodes.len(), 1);
        assert!(shell.chrome.inspector_open);
        assert_eq!(
            (
                shell.state.read(cx).snapshot.graph.candidate_revision,
                shell.state.read(cx).pending_request,
            ),
            before,
            "locating a node must not edit the graph or dispatch a request"
        );
    });
    finish_service(runtime);
}

/// `reveal_node` on a node that is gone does nothing at all — no page
/// switch, no selection change.
#[cfg(feature = "gpui-test")]
#[gpui_kit::test]
fn revealing_a_vanished_node_is_a_no_op(cx: &mut gpui_kit::TestAppContext) {
    use crate::ui::pages::Page;
    use crate::ui::shell::test_harness::{finish_service, test_shell};
    let (view, cx, runtime) = test_shell(cx);
    let node = view.read_with(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.nodes()[0].id()
    });
    view.update(cx, |shell, cx| {
        shell.set_page(Page::Devices, cx);
        // Drop the node from the snapshot the shell reads, as a removal
        // racing the click would.
        let mut snapshot = (*shell.state.read(cx).snapshot).clone();
        std::sync::Arc::make_mut(&mut snapshot.graph)
            .nodes
            .retain(|entry| entry.id != node);
        shell.state.update(cx, |state, _| {
            state.snapshot = std::sync::Arc::new(snapshot)
        });
        shell.reveal_node(node, cx);
        assert_eq!(shell.presentation.page(), Page::Devices);
        assert_eq!(shell.selection.node, None);
    });
    finish_service(runtime);
}
