#![cfg(feature = "gpui-test")]

//! Headless UI acceptance tests on gpui-kit's test support: render the real
//! shell pages and drive them through the same element APIs the platform uses.

// The gpui test macro emits `gpui::…` paths; alias the Kit facade so they
// resolve to the same GPUI snapshot the app uses.
extern crate gpui_kit as gpui;

mod common;

#[cfg(windows)]
#[path = "ui_acceptance/export_lifecycle.rs"]
mod export_lifecycle;

#[cfg(windows)]
#[path = "ui_acceptance/native_live.rs"]
mod native_live;

// Import explicitly: glob-importing the Kit would shadow Rust's #[test]
// with GPUI's test macro and recurse inside the generated body.
#[allow(unused_imports)]
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, WindowOptions, px, size};
use moiren_app::ui::UiState;
use moiren_app::ui::{Page, PageCell};
use std::time::Duration;

#[cfg(windows)]
#[gpui_kit::test]
fn provider_diagnostics_render_last_observations_and_missing_evidence(
    cx: &mut gpui_kit::TestAppContext,
) {
    use moiren_app::service::{
        ExportDiagnostics, ExportDirection, ExportIntent, ExportResourceState, ExportStatus,
        SessionPhase,
    };
    use moiren_engine::node::VirtualEndpointId;
    use moiren_windows_audio::vad::VadPcmTelemetrySnapshot;

    let runtime = common::service();
    let mut snapshot = (*common::wait_snapshot(&runtime.handle())).clone();
    snapshot.phase = SessionPhase::Idle;
    let endpoint = VirtualEndpointId::from_bytes([7; 16]);
    let mut status = ExportStatus {
        intent: ExportIntent {
            endpoint_id: endpoint,
            direction: ExportDirection::Playback,
            name: "Requested playback".into(),
            channels: 2,
            enabled: true,
            binding: None,
        },
        actual_name: Some("Observed playback".into()),
        rename_pending: true,
        resource: ExportResourceState::Present,
        allow_default: false,
        gate_open: false,
        active_revision: None,
        diagnostics: Some(ExportDiagnostics {
            generation: 42,
            native_stamp: Some([1, 2, 3, 0, 5]),
            transport_prepared: false,
            telemetry: Some(VadPcmTelemetrySnapshot {
                packet_frames: 128,
                submitted_packets: (1 << 53) + 1,
                ..Default::default()
            }),
            error: Some("endpoint-specific PCM preparation refusal".into()),
            windows_name: None,
        }),
        #[cfg(feature = "vad-integration-probe")]
        native_stamp: None,
        #[cfg(feature = "vad-integration-probe")]
        observation_generation: None,
        #[cfg(feature = "vad-integration-probe")]
        native_retirement: None,
        windows_name: None,
    };
    snapshot.exports.push(status.clone());
    status.intent.endpoint_id = VirtualEndpointId::from_bytes([8; 16]);
    status.resource = ExportResourceState::Removing;
    status.diagnostics = None;
    snapshot.exports.push(status);
    let expected: Vec<_> = snapshot
        .exports
        .iter()
        .map(|status| {
            (
                gpui_kit::SharedString::from(format!(
                    "provider-export-{}",
                    status.intent.endpoint_id
                )),
                moiren_app::ui::presentation::export_diagnostic_rows(status),
            )
        })
        .collect();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(runtime.handle(), std::sync::Arc::new(snapshot)));
    let page = cx.new(|_| PageCell::default());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1440.), px(900.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx)),
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-diagnostics", cx);
        assert!(window.try_find("provider-export-diagnostics").is_some());
        for (id, rows) in &expected {
            assert!(window.try_find(id.clone()).is_some());
            assert!(!rows.is_empty(), "a real endpoint must render details");
            for (index, (key, value)) in rows.iter().enumerate() {
                let row_id = (id.clone(), index);
                for _ in 0..80 {
                    if window.find(row_id.clone()).visible() {
                        break;
                    }
                    window.scroll(
                        "settings-scroll",
                        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
                        cx,
                    );
                }
                let rendered = window.find(row_id);
                assert!(
                    rendered.visible(),
                    "{id}: {key} must be reachable by scrolling"
                );
                assert_eq!(rendered.label(), Some(format!("{key}: {value}").as_str()));
            }
        }
    })
    .unwrap();
    common::finish(runtime);
}

#[gpui_kit::test]
fn expert_input_buffers_are_explicit_drafts_with_unqualified_latency(
    cx: &mut gpui_kit::TestAppContext,
) {
    let runtime = common::service();
    let snapshot = common::wait_snapshot(&runtime.handle());
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(runtime.handle(), snapshot));
    let page = cx.new(|_| PageCell::default());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1440.), px(900.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx)),
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-transport", cx);
        assert!(window.find("transport-latency-unqualified").visible());
        assert_eq!(
            window.find("transport-latency-unqualified").label(),
            Some("Input targets are estimates. Physical end-to-end latency is unqualified.")
        );
        assert!(window.try_find("process-expert-fields").is_none());
        assert!(window.try_find("vad-expert-fields").is_none());
        // Input Buffers also contains the measured selection above these drafts.
        // Exercise real scrolling rather than assuming the section fits at once.
        for _ in 0..16 {
            let target = window.find("process-policy-1");
            let viewport = window.find("settings-scroll").bounds();
            if target.visible()
                && target.bounds().origin.y + target.bounds().size.height
                    < viewport.origin.y + viewport.size.height - px(12.)
            {
                break;
            }
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
                cx,
            );
        }
        assert!(window.find("process-policy-1").visible());
        window.click("process-policy-1", cx);
        assert!(window.try_find("process-expert-fields").is_some());
        window.click("process-policy-0", cx);
        assert!(window.try_find("process-expert-fields").is_none());
        for _ in 0..16 {
            let target = window.find("vad-policy-1");
            let viewport = window.find("settings-scroll").bounds();
            if target.visible()
                && target.bounds().origin.y + target.bounds().size.height
                    < viewport.origin.y + viewport.size.height - px(12.)
            {
                break;
            }
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
                cx,
            );
        }
        assert!(window.find("vad-policy-1").visible());
        window.click("vad-policy-1", cx);
        assert!(window.try_find("vad-expert-fields").is_some());
        window.click("close-settings", cx);
        window.click("open-settings", cx);
        assert!(
            window.try_find("vad-expert-fields").is_some(),
            "closing the modal retains the unsaved expert draft"
        );
    })
    .unwrap();
    assert_eq!(
        common::wait_snapshot(&runtime.handle()).transport_latency,
        Default::default(),
        "draft choices must not bypass Save"
    );
    common::finish(runtime);
}

#[gpui_kit::test]
fn shell_renders_statusbar_and_session_panel(cx: &mut gpui_kit::TestAppContext) {
    let runtime = common::service();
    let handle = runtime.handle();
    let snapshot = common::wait_snapshot(&handle);
    cx.update(gpui_kit::init);

    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell::default());

    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx))
        })
        .unwrap()
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("engine-statusbar").visible());
        assert!(window.find("session-io-panel").visible());
    })
    .unwrap();

    common::finish(runtime);
}

#[gpui_kit::test]
fn page_cell_switches_between_graph_and_mixer(cx: &mut gpui_kit::TestAppContext) {
    let runtime = common::service();
    let handle = runtime.handle();
    let snapshot = common::wait_snapshot(&handle);
    cx.update(gpui_kit::init);

    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell { page: Page::Mixer });

    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| moiren_app::ui::shell_for_tests(state, page.clone(), window, cx))
        })
        .unwrap()
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("mixer-page").visible());
        assert!(window.try_find("session-io-panel").is_none());

        // Switch back to Graph through the shared cell.
        page.update(cx, |cell, _| cell.page = Page::Graph);
        window.render_frame(cx);
        assert!(window.find("session-io-panel").visible());
    })
    .unwrap();

    common::finish(runtime);
}

#[gpui_kit::test]
fn window_smoke_options_valid(cx: &mut gpui_kit::TestAppContext) {
    // Keep the fixed-size test window machinery exercised.
    let runtime = common::service();
    let handle = runtime.handle();
    let snapshot = common::wait_snapshot(&handle);
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell::default());

    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::default())),
                ..Default::default()
            },
            |window, cx| {
                let _ = size(px(640.), px(480.));
                cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx))
            },
        )
        .unwrap()
    });
    let _ = window;
    common::finish(runtime);
}

#[gpui_kit::test]
fn production_navigation_opens_every_workspace_and_settings_overlays_matrix(
    cx: &mut gpui_kit::TestAppContext,
) {
    let runtime = common::service();
    let handle = runtime.handle();
    let snapshot = common::wait_snapshot(&handle);
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell::default());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1440.), px(900.)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| moiren_app::ui::shell_for_tests(state, page.clone(), window, cx))
            },
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for (tab, content) in [
            ("tab-mixer", "mixer-page"),
            ("tab-devices", "devices-page"),
            ("tab-graph", "graph-page"),
            ("tab-matrix", "matrix-page"),
        ] {
            window.click(tab, cx);
            assert!(
                window.find(content).visible(),
                "{tab} must render the real page"
            );
            assert_eq!(window.find(tab).selected(), Some(true));
            assert!(window.find("engine-statusbar").visible());
        }
        window.click("route-0-0", cx);
        assert!(window.find("disconnect-route").visible());
        window.click("open-settings", cx);
        assert!(window.find("settings-page").visible());
        assert!(window.find("settings-dialog").visible());
        assert!(window.find("settings-backdrop").visible());
        assert!(
            window.try_find("matrix-page").is_some(),
            "Settings overlays the existing workspace"
        );
        assert_eq!(page.read(cx).page, Page::Matrix);
        window.click("close-settings", cx);
        assert!(window.try_find("settings-dialog").is_none());
        assert!(window.find("matrix-page").visible());
        assert!(
            window.find("disconnect-route").visible(),
            "route selection survives Settings"
        );
        assert_eq!(page.read(cx).page, Page::Matrix);
    })
    .unwrap();
    common::finish(runtime);
}

#[gpui_kit::test]
fn compact_shell_keeps_navigation_and_opens_session_drawer(cx: &mut gpui_kit::TestAppContext) {
    let runtime = common::service();
    let handle = runtime.handle();
    let snapshot = common::wait_snapshot(&handle);
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell::default());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1024.), px(720.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx)),
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // Panels stay mounted and animate their width; closed means not
        // visible rather than absent.
        assert!(!window.find("session-io-panel").visible());
        for tab in [
            "tab-graph",
            "tab-mixer",
            "tab-matrix",
            "tab-devices",
            "open-settings",
        ] {
            assert!(window.find(tab).visible());
        }
        window.click("library-toggle", cx);
        // The library springs open on its edge handle; panels animate their
        // width, so give the spring real time before asserting visibility.
        std::thread::sleep(std::time::Duration::from_millis(900));
        window.render_frame(cx);
        assert!(window.find("session-io-panel").visible());
        window.click("inspector-toggle", cx);
        std::thread::sleep(std::time::Duration::from_millis(900));
        window.render_frame(cx);
        assert!(
            !window.find("session-io-panel").visible(),
            "compact drawers are exclusive"
        );
        window.click("tab-devices", cx);
        assert!(window.find("devices-page").visible());
    })
    .unwrap();
    common::finish(runtime);
}

#[gpui_kit::test]
fn large_matrix_keeps_both_headers_visible_while_scrolling(cx: &mut gpui_kit::TestAppContext) {
    let runtime = common::service();
    let handle = runtime.handle();
    let mut projection = UiState::new(handle.clone(), common::wait_snapshot(&handle));
    for _ in 0..16 {
        projection.dispatch(moiren_app::service::AppRequest::EditGraph(
            moiren_app::service::GraphEdit::AddNode {
                kind: moiren_core::graph::NodeKind::Gain,
                channels: 2,
            },
        ));
    }
    common::wait_until(Duration::from_secs(2), || {
        projection.poll();
        !projection.pending_request
    });
    assert_eq!(projection.snapshot.graph.graph.nodes().len(), 20);
    cx.update(gpui_kit::init);
    let state = cx.new(|_| projection);
    let page = cx.new(|_| PageCell { page: Page::Matrix });
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1024.), px(720.)),
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| moiren_app::ui::shell_for_tests(state, page, window, cx)),
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let column_y = window.find("matrix-column-header").bounds().origin.y;
        let row_x = window.find("matrix-row-header").bounds().origin.x;
        window.scroll(
            "matrix-grid",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-400.), px(-300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("matrix-column-header").visible());
        assert!(window.find("matrix-row-header").visible());
        assert_eq!(
            window.find("matrix-column-header").bounds().origin.y,
            column_y
        );
        assert_eq!(window.find("matrix-row-header").bounds().origin.x, row_x);
    })
    .unwrap();
    common::finish(runtime);
}
