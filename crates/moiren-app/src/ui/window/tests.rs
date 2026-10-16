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
            shell.mixer.width = 210.;
            shell.mixer.dock_width = 410.;
            shell.mixer.order.reverse();
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
                assert_eq!(shell.mixer.width, 210.);
                assert_eq!(shell.mixer.dock_width, 410.);
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
fn app_quit_drains_delayed_preference_chain_and_restores_latest_profile(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::{project, service::AudioLatencyProfile};
    use std::{
        sync::{Arc, Mutex},
        task::{Poll, Waker},
    };
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
    let directory = std::env::temp_dir().join(format!(
        "moiren-latency-quit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = directory.join("preferences.json");
    project::save_preferences_at(&path, &project::AppPreferences::default()).unwrap();
    let gate = Arc::new(Mutex::new((false, None::<Waker>)));
    let blocked = gate.clone();
    view.update(cx, |shell, cx| {
        shell.preferences = project::AppPreferences::default();
        shell.preferences_path = Some(path.clone());
        shell.settings_open = true;
        // A parked predecessor deterministically delays the first actual save
        // without blocking the UI executor or relying on filesystem speed.
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
        cx.notify();
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("latency-balanced", cx);
        window.click("latency-low", cx);
    });
    cx.run_until_parked();
    let quitting = view.update(cx, |shell, _| {
        assert_eq!(
            shell.preferences.latency_profile,
            AudioLatencyProfile::Low,
            "both profile clicks were accepted before shutdown"
        );
        assert!(shell.preference_save.is_some());
        shell.preferences_quitting.clone()
    });
    assert!(gate.lock().unwrap().1.is_some(), "save chain is parked");
    assert_eq!(
        project::load_preferences_at(&path).unwrap().latency_profile,
        AudioLatencyProfile::Compatible
    );
    // Release only from the shutdown future, after quit has transferred the
    // pending chain and cleared the windows. A detached task alone is insufficient.
    let release = gate.clone();
    cx.update(|_, cx| {
        cx.on_app_quit(move |_| {
            let gate = release.clone();
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
    // Invoke GPUI's real shutdown directly. TestAppContext::quit also destroys
    // the leaked VisualTestContext itself and is reserved for test teardown.
    let app = cx.app.clone();
    drop(view);
    app.borrow_mut().shutdown();
    assert!(
        quitting.load(std::sync::atomic::Ordering::Acquire),
        "shell transferred its pending write at quit"
    );
    assert!(gate.lock().unwrap().0, "shutdown polled its quit futures");
    let restored = project::load_preferences_at(&path).unwrap();
    assert_eq!(restored.latency_profile, AudioLatencyProfile::Low);
    handle.request_exit();
    runtime.join().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[gpui_kit::test]
fn settings_profile_actions_reach_service_and_persist_last_choice(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::{project, service::AudioLatencyProfile};
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
    let directory = std::env::temp_dir().join(format!(
        "moiren-latency-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = directory.join("preferences.json");
    view.update(cx, |shell, cx| {
        shell.preferences = project::AppPreferences::default();
        shell.preferences_path = Some(path.clone());
        shell.settings_open = true;
        cx.notify();
    });
    for (id, expected) in [
        ("latency-balanced", AudioLatencyProfile::Balanced),
        ("latency-low", AudioLatencyProfile::Low),
        ("latency-compatible", AudioLatencyProfile::Compatible),
    ] {
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        });
        cx.run_until_parked();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let consumed = view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                let state = shell.state.read(cx);
                !state.pending_request && state.snapshot.latency_profile == expected
            });
            if consumed {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            project::load_preferences_at(&path).unwrap().latency_profile,
            expected
        );
        view.update(cx, |shell, cx| {
            assert_eq!(shell.project_message.as_deref(), Some("Preferences saved"));
            assert_eq!(
                shell.state.read(cx).snapshot.phase,
                crate::service::SessionPhase::Idle
            );
            assert_eq!(shell.state.read(cx).snapshot.active_latency_profile, None);
        });
    }
    // Both writes are queued before the executor runs; their order must survive
    // background filesystem work, rather than depending on task scheduling.
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("latency-balanced", cx);
        window.click("latency-low", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        project::load_preferences_at(&path).unwrap().latency_profile,
        AudioLatencyProfile::Low
    );
    let blocked_path = directory.join("blocked");
    std::fs::write(&blocked_path, b"a file cannot be a configuration directory").unwrap();
    view.update(cx, |shell, cx| {
        shell.preferences_path = Some(blocked_path.join("preferences.json"));
        shell.save_preferences(cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, cx| {
        assert!(
            shell
                .project_message
                .as_deref()
                .unwrap()
                .starts_with("Could not save preferences:")
        );
        shell.preferences_path = Some(path.clone());
        shell.save_preferences(cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, _| {
        assert_eq!(shell.project_message.as_deref(), Some("Preferences saved"))
    });
    handle.request_exit();
    runtime.join().unwrap();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("latency-compatible", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, cx| {
        assert_eq!(shell.preferences.latency_profile, AudioLatencyProfile::Low);
        assert!(shell.state.read(cx).dispatch_error.is_some());
    });
    assert_eq!(
        project::load_preferences_at(&path).unwrap().latency_profile,
        AudioLatencyProfile::Low
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&blocked_path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[gpui_kit::test]
fn graph_wire_selection_shows_connection_properties_without_selecting_node(
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
        window.render_frame(cx);
        assert!(window.find("matrix-inspector").visible());
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
