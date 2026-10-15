use super::{ServiceRuntime, StudioShell, UiState, pages, theme};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, point, px};

#[gpui_kit::test]
fn toolbar_reserves_draggable_space_outside_interactive_controls(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (_view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        for id in ["titlebar-drag-leading", "titlebar-drag-trailing"] {
            let region = window.find(id);
            assert!(region.visible());
            assert!(region.bounds().size.width >= px(20.0));
        }
        window.click("tab-mixer", cx);
        window.render_frame(cx);
        assert!(window.find("tab-mixer").visible());
    });
    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn close_releases_window_view_and_reopens_presentation_without_stopping_service(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let desktop = cx.new(|cx| super::desktop::Desktop::new(state.clone(), None, cx));
    cx.update(|cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.state_path = None;
            desktop.saved = crate::project::WindowState::default();
            desktop.saved.width = 1024.0;
            desktop.show_window(cx).unwrap();
        });
    });
    let window = cx.windows()[0];
    cx.update(|cx| {
        window
            .update(cx, |_, window, cx| window.render_frame(cx))
            .unwrap()
    });
    let weak = cx.update(|cx| {
        let weak = desktop.read(cx).shell.clone().unwrap();
        weak.update(cx, |shell, _| {
            shell.presentation.page = pages::Page::Mixer;
            shell.camera.pan_x = 84.0;
            shell.sidebar_open = true;
            shell.inspector_open = true;
            shell.device_tab = super::devices::DeviceTab::Inputs;
            shell.selected_device = Some("selected-input".into());
            shell
                .node_positions
                .insert("test-node".into(), (12.0, 34.0));
            shell.project_name = "Keep candidate".into();
        })
        .unwrap();
        window
            .update(cx, |_, window, cx| {
                weak.update(cx, |shell, cx| shell.close_window(window, cx))
                    .unwrap();
            })
            .unwrap();
        weak
    });
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert!(
        weak.upgrade().is_none(),
        "closed shell must not be retained"
    );
    cx.update(|cx| {
        assert_ne!(
            state.read(cx).snapshot.phase,
            crate::service::SessionPhase::Exited
        );
        desktop.update(cx, |desktop, cx| desktop.show_window(cx).unwrap());
        let reopened = desktop.read(cx).shell.clone().unwrap();
        cx.windows()[0]
            .update(cx, |_, window, cx| window.render_frame(cx))
            .unwrap();
        reopened
            .update(cx, |shell, _| {
                assert_eq!(shell.presentation.page, pages::Page::Mixer);
                assert_eq!(shell.camera.pan_x, 84.0);
                assert_eq!(shell.node_positions.get("test-node"), Some(&(12.0, 34.0)));
                assert_eq!(shell.project_name, "Keep candidate");
                assert!(
                    shell.sidebar_open && shell.inspector_open,
                    "reopen must preserve manually opened compact drawers"
                );
                assert!(matches!(
                    shell.device_tab,
                    super::devices::DeviceTab::Inputs
                ));
                assert_eq!(shell.selected_device.as_deref(), Some("selected-input"));
            })
            .unwrap();
    });
    assert_eq!(cx.windows().len(), 1);
    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn closed_window_keeps_preference_writes_until_application_shutdown(
    cx: &mut gpui_kit::TestAppContext,
) {
    use std::{
        sync::{Arc, Mutex},
        task::{Poll, Waker},
    };

    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let desktop = cx.new(|cx| super::desktop::Desktop::new(state, None, cx));
    cx.update(|cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.state_path = None;
            desktop.saved = crate::project::WindowState::default();
            desktop.show_window(cx).unwrap();
        });
    });
    let directory = std::env::temp_dir().join(format!(
        "moiren-closed-window-preferences-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = directory.join("preferences.json");
    let gate = Arc::new(Mutex::new((false, None::<Waker>)));
    let blocked = gate.clone();
    let window = cx.windows()[0];
    cx.update(|cx| {
        let weak = desktop.read(cx).shell.clone().unwrap();
        weak.update(cx, |shell, cx| {
            shell.preferences_path = Some(path.clone());
            shell.preference_save = Some(cx.background_executor().spawn(async move {
                std::future::poll_fn(move |cx| {
                    let mut state = blocked.lock().unwrap();
                    if state.0 {
                        Poll::Ready(())
                    } else {
                        state.1 = Some(cx.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
            }));
            shell.preferences.density = crate::project::UiDensity::Compact;
            shell.save_preferences(cx);
            shell.preferences.density = crate::project::UiDensity::Comfortable;
            shell.save_preferences(cx);
        })
        .unwrap();
        window
            .update(cx, |_, window, cx| {
                weak.update(cx, |shell, cx| shell.close_window(window, cx))
                    .unwrap()
            })
            .unwrap();
    });
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert!(!path.exists(), "writes must remain behind the predecessor");
    cx.update(|cx| {
        cx.on_app_quit(move |_| {
            let gate = gate.clone();
            async move {
                let mut state = gate.lock().unwrap();
                state.0 = true;
                if let Some(waker) = state.1.take() {
                    waker.wake();
                }
            }
        })
        .detach();
    });
    cx.app.borrow_mut().shutdown();
    assert_eq!(
        crate::project::load_preferences_at(&path).unwrap().density,
        crate::project::UiDensity::Comfortable,
        "shutdown must drain the latest write after the window is destroyed"
    );
    handle.request_exit();
    runtime.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui_kit::test]
fn graph_uses_full_canvas_and_wire_selection_opens_real_route_inspector(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    });
    let position = view.update(cx, |shell, cx| {
        let (width, height) = shell.canvas_size.expect("canvas has completed layout");
        assert!(
            width > 500. && height > 300.,
            "the drawing surface must fill the workspace"
        );
        assert!(
            shell.camera.zoom > 0.5,
            "Fit must leave this four-node graph readable"
        );
        let world = pages::graph_canvas::CanvasWorld::positioned(
            &shell.state.read(cx).snapshot,
            &shell.node_positions,
        );
        let wire = &world.wires[0];
        point(
            px(if shell.sidebar_open {
                theme::LIBRARY_WIDTH
            } else {
                0.
            } + shell.camera.pan_x
                + (wire.from.0 + wire.to.0) * 0.5 * shell.camera.zoom),
            px(48. + shell.camera.pan_y + wire.from.1 * shell.camera.zoom),
        )
    });
    cx.update(|window, cx| {
        window.click_at("graph-page", position, cx);
        assert!(window.find("disconnect-route").visible());
    });
    view.update(cx, |shell, cx| {
        assert!(shell.selected_route.is_some());
        assert!(shell.selected_node.is_none());
        assert_eq!(
            shell.state.read(cx).snapshot.graph.graph.edges().len(),
            3,
            "selection never mutates topology"
        );
    });
    handle.request_exit();
    runtime.join().unwrap();
}
