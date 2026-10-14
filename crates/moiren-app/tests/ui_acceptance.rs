#![cfg(feature = "gpui-test")]

//! Headless UI acceptance tests on gpui-kit's test support: render the real
//! shell pages and drive them through the same element APIs the platform uses.

// The gpui test macro emits `gpui::…` paths; alias the Kit facade so they
// resolve to the same GPUI snapshot the app uses.
extern crate gpui_kit as gpui;

// Import explicitly: glob-importing the Kit would shadow Rust's #[test]
// with GPUI's test macro and recurse inside the generated body.
#[allow(unused_imports)]
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, WindowOptions, px, size};
use moiren_app::service::ServiceRuntime;
use moiren_app::ui::UiState;
use moiren_app::ui::{Page, PageCell};

#[gpui_kit::test]
fn shell_renders_statusbar_and_session_panel(cx: &mut gpui_kit::TestAppContext) {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = handle.try_snapshot().unwrap();
    cx.update(gpui_kit::init);

    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell::default());

    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|_| moiren_app::ui::shell_for_tests(state, page, window))
        })
        .unwrap()
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("engine-statusbar").visible());
        assert!(window.find("session-io-panel").visible());
    })
    .unwrap();

    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn page_cell_switches_between_graph_and_mixer(cx: &mut gpui_kit::TestAppContext) {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = handle.try_snapshot().unwrap();
    cx.update(gpui_kit::init);

    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| PageCell { page: Page::Mixer });

    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|_| moiren_app::ui::shell_for_tests(state, page.clone(), window))
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

    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn window_smoke_options_valid(cx: &mut gpui_kit::TestAppContext) {
    // Keep the fixed-size test window machinery exercised.
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = handle.try_snapshot().unwrap();
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
                cx.new(|_| moiren_app::ui::shell_for_tests(state, page, window))
            },
        )
        .unwrap()
    });
    let _ = window;
    handle.request_exit();
    runtime.join().unwrap();
}
