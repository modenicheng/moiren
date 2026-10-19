#![cfg(feature = "gpui-test")]

//! Headless UI acceptance tests on gpui-kit's test support: render the real
//! shell pages and drive them through the same element APIs the platform uses.

// The gpui test macro emits `gpui::…` paths; alias the Kit facade so they
// resolve to the same GPUI snapshot the app uses.
extern crate gpui_kit as gpui;

mod common;

// Import explicitly: glob-importing the Kit would shadow Rust's #[test]
// with GPUI's test macro and recurse inside the generated body.
#[allow(unused_imports)]
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, WindowOptions, px, size};
use moiren_app::ui::UiState;
use moiren_app::ui::{Page, PageCell};
use std::time::Duration;

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
        assert!(window.try_find("session-io-panel").is_none());
        for tab in [
            "tab-graph",
            "tab-mixer",
            "tab-matrix",
            "tab-devices",
            "open-settings",
        ] {
            assert!(window.find(tab).visible());
        }
        window.click("toggle-library", cx);
        assert!(window.find("session-io-panel").visible());
        window.click("toggle-inspector", cx);
        assert!(
            window.try_find("session-io-panel").is_none(),
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
