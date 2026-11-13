use super::{ServiceRuntime, StudioShell, UiState, pages, theme};
use crate::ui::shell::test_harness::{
    finish_service, initial_projection, settle_panels, shell_with_state, spawn_service, test_shell,
    wait_for, wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, point, px};
use std::time::Duration;

/// The control pump hands a published snapshot to the UI as soon as the
/// service publishes it, not on its next timer tick (design 22 D7). The test
/// executor never fires the pump's 33 ms timer unless its clock is advanced,
/// so the snapshot below can only reach the state through the publish wake —
/// the path that made every toggle and checkbox wait up to a whole tick.
/// The test thread plays the service (a real service thread waking the pump
/// would trip the test scheduler's determinism check).
#[gpui_kit::test]
fn the_pump_delivers_a_publish_without_waiting_for_its_timer(cx: &mut gpui_kit::TestAppContext) {
    let (handle, mailbox) = crate::service::ServiceMailbox::new(std::thread::current());
    let initial = handle.try_snapshot().expect("initial snapshot");
    let state = cx.new(|_| UiState::new(handle.clone(), initial.clone()));
    let (view, cx) = shell_with_state(cx, state);
    view.update(cx, |shell, cx| shell.spawn_poll(cx));
    cx.run_until_parked();
    let mut published = (*initial).clone();
    published.elapsed = Duration::from_secs(7);
    mailbox.publish(published);
    cx.run_until_parked();
    view.read_with(cx, |shell, cx| {
        assert_eq!(
            shell.state.read(cx).snapshot.elapsed,
            Duration::from_secs(7),
            "the publish reached the UI without the pump's timer firing"
        );
    });
}

#[gpui_kit::test]
fn toolbar_reserves_draggable_space_outside_interactive_controls(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (_view, cx, runtime) = test_shell(cx);
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
    finish_service(runtime);
}

#[gpui_kit::test]
fn close_releases_window_view_and_reopens_presentation_without_stopping_service(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
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
        let test_node = state.read(cx).snapshot.graph.graph.nodes()[0].id();
        weak.update(cx, |shell, _| {
            shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Mixer);
            shell.graph_view.camera.pan_x = 84.0;
            shell.chrome.sidebar_open = true;
            shell.chrome.inspector_open = true;
            shell.devices.device_tab = super::devices::DeviceTab::Inputs;
            shell.devices.selected_device = Some("selected-input".into());
            shell
                .graph_view
                .node_positions
                .insert(test_node, (12.0, 34.0));
            shell.project_io.name = "Keep candidate".into();
            shell.mixer.presentation.order.reverse();
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
            .update(cx, |shell, cx| {
                let test_node = state.read(cx).snapshot.graph.graph.nodes()[0].id();
                assert_eq!(shell.presentation.page(), pages::Page::Mixer);
                assert_eq!(shell.graph_view.camera.pan_x, 84.0);
                assert_eq!(
                    shell.graph_view.node_positions.get(&test_node),
                    Some(&(12.0, 34.0))
                );
                assert_eq!(shell.project_io.name, "Keep candidate");
                assert!(
                    shell.chrome.sidebar_open && shell.chrome.inspector_open,
                    "reopen must preserve manually opened compact drawers"
                );
                assert!(matches!(
                    shell.devices.device_tab,
                    super::devices::DeviceTab::Inputs
                ));
                assert_eq!(
                    shell.devices.selected_device.as_deref(),
                    Some("selected-input")
                );
            })
            .unwrap();
    });
    assert_eq!(cx.windows().len(), 1);
    finish_service(runtime);
}

/// The compact breakpoint adaptation is enter-only (design 21 §2): crossing
/// below [`theme::COMPACT_BREAKPOINT`] collapses the side panels, but growing
/// back above it must leave them exactly as the compact session left them.
/// The old transition also force-restored `sidebar_open`/`inspector_open` on
/// exit, reopening panels the user had closed — before shrinking, or while
/// compact — and re-clobbering both flags on every rapid crossing.
#[gpui_kit::test]
fn compact_breakpoint_collapses_on_enter_and_exit_leaves_panels_alone(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    // `simulate_resize` drives the real viewport-size callback the shell's
    // breakpoint check reads; the harness `window.resize` only moves bounds.
    cx.simulate_resize(gpui_kit::size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert!(!shell.chrome.compact, "1440 sits above the breakpoint");
        assert!(shell.chrome.sidebar_open && shell.chrome.inspector_open);
    });
    // Entering compact collapses both panels.
    cx.simulate_resize(gpui_kit::size(px(1024.), px(720.)));
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert!(shell.chrome.compact, "1024 sits below the breakpoint");
        assert!(!shell.chrome.sidebar_open && !shell.chrome.inspector_open);
    });
    // Leaving compact restores nothing: the panels stay closed.
    cx.simulate_resize(gpui_kit::size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert!(!shell.chrome.compact);
        assert!(
            !shell.chrome.sidebar_open && !shell.chrome.inspector_open,
            "the exit transition must not force the panels open"
        );
    });
    // A second entry re-applies the collapse, and a user arrangement made
    // while compact (the rail handle re-opens the library, the inspector
    // stays shut) survives the next exit unchanged.
    cx.simulate_resize(gpui_kit::size(px(1024.), px(720.)));
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert!(shell.chrome.compact);
        assert!(!shell.chrome.sidebar_open && !shell.chrome.inspector_open);
        shell.chrome.sidebar_open = true;
    });
    cx.simulate_resize(gpui_kit::size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert!(!shell.chrome.compact);
        assert!(
            shell.chrome.sidebar_open,
            "the user-opened library stays open"
        );
        assert!(
            !shell.chrome.inspector_open,
            "the closed inspector stays closed"
        );
    });
    finish_service(runtime);
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
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
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
            shell.preferences.path = Some(path.clone());
            shell.preferences.save = Some(cx.background_executor().spawn(async move {
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
            shell.preferences.values.density = crate::project::UiDensity::Compact;
            shell.save_preferences(cx);
            shell.preferences.values.density = crate::project::UiDensity::Comfortable;
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
    finish_service(runtime);
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
    let (view, cx, runtime) = test_shell(cx);
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
        shell.preferences.values = project::AppPreferences::default();
        shell.preferences.path = Some(path.clone());
        // A parked predecessor deterministically delays the first actual save
        // without blocking the UI executor or relying on filesystem speed.
        shell.preferences.save = Some(cx.background_executor().spawn(async move {
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
    view.update(cx, |shell, cx| {
        shell.preferences.values.latency_profile = AudioLatencyProfile::Balanced;
        shell.save_preferences(cx);
        shell.preferences.values.latency_profile = AudioLatencyProfile::Low;
        shell.save_preferences(cx);
    });
    cx.run_until_parked();
    let quitting = view.update(cx, |shell, _| {
        assert_eq!(
            shell.preferences.values.latency_profile,
            AudioLatencyProfile::Low,
            "both profile clicks were accepted before shutdown"
        );
        assert!(shell.preferences.save.is_some());
        shell.preferences.quitting.clone()
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
    finish_service(runtime);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[gpui_kit::test]
fn graph_wire_selection_shows_connection_properties_without_selecting_node(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    });
    let position = view.update(cx, |shell, cx| {
        let (width, height) = shell
            .graph_view
            .canvas_size
            .expect("canvas has completed layout");
        assert!(
            width > 500. && height > 300.,
            "the drawing surface must fill the workspace"
        );
        assert!(
            shell.graph_view.camera.zoom > 0.5,
            "Fit must leave this four-node graph readable"
        );
        let world = pages::graph::graph_canvas::CanvasWorld::positioned(
            &shell.state.read(cx).snapshot,
            &shell.graph_view.node_positions,
        );
        let wire = &world.wires[0];
        point(
            px(if shell.chrome.sidebar_open {
                theme::LIBRARY_WIDTH
            } else {
                crate::ui::theme::LIBRARY_RAIL_WIDTH
            } + shell.graph_view.camera.pan_x
                + (wire.from.0 + wire.to.0) * 0.5 * shell.graph_view.camera.zoom),
            px(shell.graph_view.camera.pan_y + wire.from.1 * shell.graph_view.camera.zoom),
        )
    });
    cx.update(|window, cx| {
        // First click selects the wire; the second opens the inspector.
        window.click_at("graph-page", position, cx);
        window.click_at("graph-page", position, cx);
        window.render_frame(cx);
        assert!(window.find("matrix-inspector").visible());
        assert!(window.find("disconnect-route").visible());
    });
    view.update(cx, |shell, cx| {
        assert!(shell.selection.route.is_some());
        assert!(shell.selection.node.is_none());
        assert_eq!(
            shell.state.read(cx).snapshot.graph.graph.edges().len(),
            3,
            "selection never mutates topology"
        );
    });
    finish_service(runtime);
}

fn settings_test_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!(
            "moiren-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .join("preferences.json")
}

fn settle_settings(view: &gpui_kit::Entity<StudioShell>, cx: &mut gpui_kit::VisualTestContext) {
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        cx.update(|window, cx| window.render_frame(cx));
        cx.run_until_parked();
        view.update(cx, |shell, _| !shell.settings.editor.busy())
    });
    cx.run_until_parked();
}

/// The audio actions live at the very bottom of the scrolled settings
/// column, so a click needs the page scrolled to its end first — the
/// headless hit test does not clip to the window, which would otherwise let
/// a click on an off-screen button pass silently.
fn scroll_settings_to_bottom(cx: &mut gpui_kit::VisualTestContext) {
    cx.update(|window, cx| {
        window.scroll(
            "settings-scroll",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        );
        window.render_frame(cx);
    });
}

#[gpui_kit::test]
fn settings_page_replaces_workspace_and_restores_it_on_close(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        shell.graph_view.camera.pan_x = 123.;
        shell.graph_view.camera.pan_y = 87.;
        shell.graph_view.camera.zoom = 0.89;
        shell.graph_view.camera.adjusted = true;
        shell.selection.node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[1].id());
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        assert!(window.find("settings-page").visible());
        assert!(
            window.try_find("graph-page").is_none(),
            "settings replaces the workspace while open"
        );
        assert!(
            window.find("settings-save").visible(),
            "the audio footer stays visible"
        );
        window.press("delete", cx);
        for _ in 0..80 {
            window.press("tab", cx);
        }
        window.scroll(
            "settings-scroll",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-160.))),
            cx,
        );
    });
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            assert!(
                shell.settings.editor.focus.contains_focused(window, cx),
                "Tab remains inside the page"
            );
            assert_eq!(shell.state.read(cx).snapshot.graph.graph.nodes().len(), 4);
            assert!(shell.selection.node.is_some());
            assert_eq!(
                (
                    shell.graph_view.camera.pan_x,
                    shell.graph_view.camera.pan_y,
                    shell.graph_view.camera.zoom
                ),
                (123., 87., 0.89)
            );
        });
        window.press("escape", cx);
        assert!(window.try_find("settings-page").is_none());
        assert!(window.find("graph-page").visible());
        view.update(cx, |shell, cx| {
            assert_eq!(shell.presentation.page(), pages::Page::Graph);
            assert!(!shell.presentation.workspace.is_settings());
            assert!(!shell.settings.editor.focus.contains_focused(window, cx));
            assert_eq!(
                (
                    shell.graph_view.camera.pan_x,
                    shell.graph_view.camera.pan_y,
                    shell.graph_view.camera.zoom
                ),
                (123., 87., 0.89),
                "workspace state survives the settings round trip"
            );
        });
        // The gear toggles the page.
        window.click("open-settings", cx);
        assert!(window.find("settings-page").visible());
        window.click("open-settings", cx);
        assert!(window.try_find("settings-page").is_none());
        // A workspace tab switches away and closes the page.
        window.click("open-settings", cx);
        window.click("tab-mixer", cx);
        assert!(window.try_find("settings-page").is_none());
        assert!(window.find("mixer-page").visible());
        assert_eq!(window.find("tab-graph").selected(), Some(false));
        window.click("tab-graph", cx);
        assert!(window.find("graph-page").visible());
        window.click("open-settings", cx);
        window.click("close-settings", cx);
        assert!(window.try_find("settings-page").is_none());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn settings_controls_are_drafts_until_save_and_stopped_save_never_starts_audio(
    cx: &mut gpui_kit::TestAppContext,
) {
    use moiren_engine::audio_config::{AudioBackend, ProcessingPrecision};
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("draft");
    view.update(cx, |shell, cx| {
        shell.preferences.values = crate::project::AppPreferences::default();
        shell.preferences.path = Some(path.clone());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        assert!(window.find("settings-page").visible());
        window.click("processing-rate-0", cx);
        window.render_frame(cx);
        window.click("processing-precision-1", cx);
        window.render_frame(cx);
        window.click("processing-buffer-trigger", cx);
        window.render_frame(cx);
        window.click("settings-menu-item-6", cx);
        window.render_frame(cx);
        window.click("audio-backend-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("default-input-mode-0").is_none());
        assert!(window.try_find("default-input-format-0").is_none());
        assert!(window.try_find("settings-restart").is_none());
    });
    view.update(cx, |shell, cx| {
        assert!(shell.settings.editor.global.dirty());
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .audio_settings
                .processing
                .sample_rate,
            48_000
        );
        assert_eq!(
            shell.preferences.values.audio.processing.sample_rate,
            48_000
        );
        assert!(!shell.state.read(cx).pending_request);
    });
    assert!(!path.exists());
    scroll_settings_to_bottom(cx);
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert_eq!(state.snapshot.phase, crate::service::SessionPhase::Idle);
        assert!(state.snapshot.active_audio_settings.is_none());
        assert_eq!(state.snapshot.audio_settings.backend, AudioBackend::Asio);
        assert_eq!(
            state.snapshot.audio_settings.processing.precision,
            ProcessingPrecision::F64
        );
        assert_eq!(state.snapshot.audio_settings.processing.block_frames, 4096);
        assert!(!shell.settings.editor.global.dirty());
        assert_eq!(
            shell.project_io.message,
            Some(crate::ui::shell::project::ProjectMessage::PreferencesSaved),
            "the completed preferences write reports its typed outcome"
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .processing
            .sample_rate,
        44_100
    );
    // Density is an immediate preference: it applies on the click and
    // persists through the quiet write chain, with no Save in the path.
    cx.update(|window, cx| {
        window.click("settings-nav-interface", cx);
        window.click("density-1", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, _| {
        assert_eq!(shell.chrome.density, super::Density::Compact);
        assert!(!shell.settings.editor.global.dirty());
        assert!(
            shell
                .settings
                .editor
                .global
                .feedback
                .as_ref()
                .is_none_or(|feedback| !feedback.is_error())
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path).unwrap().density,
        crate::project::UiDensity::Compact
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// Meter ballistics moved from the mixer context menu into the Interface
/// page; the click applies immediately and persists through the quiet write
/// chain, same as before.
#[gpui_kit::test]
fn meter_preferences_apply_immediately_from_the_interface_page(cx: &mut gpui_kit::TestAppContext) {
    use crate::project::{MeterFalloff, PeakHold};
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("meters");
    view.update(cx, |shell, cx| {
        shell.preferences.values = crate::project::AppPreferences::default();
        shell.preferences.path = Some(path.clone());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-interface", cx);
        window.render_frame(cx);
        window.click("meter-falloff-0", cx);
        window.render_frame(cx);
        window.click("peak-hold-0", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, _| {
        assert_eq!(
            shell.preferences.values.meters.falloff,
            MeterFalloff::Moderate
        );
        assert_eq!(shell.preferences.values.meters.peak_hold, PeakHold::Off);
        assert!(!shell.settings.editor.global.dirty());
    });
    let saved = crate::project::load_preferences_at(&path).unwrap();
    assert_eq!(saved.meters.falloff, MeterFalloff::Moderate);
    assert_eq!(saved.meters.peak_hold, PeakHold::Off);
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// The restore-on-launch switch has no other UI; the toggle applies
/// immediately and persists like the other Interface preferences.
#[gpui_kit::test]
fn restore_last_project_switch_applies_immediately(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("restore");
    view.update(cx, |shell, cx| {
        shell.preferences.values = crate::project::AppPreferences::default();
        shell.preferences.path = Some(path.clone());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-interface", cx);
        window.render_frame(cx);
        window.click("settings-restore-project", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, _| {
        assert!(shell.preferences.values.restore_last_project);
    });
    assert!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .restore_last_project
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn device_inspection_and_overrides_are_independent_from_candidate_bindings(
    cx: &mut gpui_kit::TestAppContext,
) {
    use super::settings::{Direction, Editor};
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    std::sync::Arc::make_mut(&mut snapshot.catalog)
        .outputs
        .push(crate::service::DeviceRow {
            endpoint_id: "output-a".into(),
            name: "Studio output".into(),
        });
    let state = cx.new(|_| UiState::new(handle.clone(), std::sync::Arc::new(snapshot)));
    let (view, cx) = shell_with_state(cx, state);
    let path = settings_test_path("device");
    view.update(cx, |shell, cx| {
        shell.preferences.path = Some(path.clone());
        shell.set_page(pages::Page::Devices, cx);
    });
    let before = view.update(cx, |shell, cx| {
        (
            shell.state.read(cx).snapshot.graph.clone(),
            shell.state.read(cx).snapshot.phase,
        )
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        // The device row itself opens the detail view (design 23 §3.2).
        window.click("device-row-output-a", cx);
        window.click("device-override", cx);
        window.click("device-output-rate-trigger", cx);
        window.render_frame(cx);
        window.click("settings-menu-item-4", cx);
        window.render_frame(cx);
        window.click("device-output-format-trigger", cx);
        window.render_frame(cx);
        window.click("settings-menu-item-2", cx);
        window.render_frame(cx);
        window.click("device-settings-save", cx);
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        // Design 23: there is no selection ledger for an override to touch;
        // the graph itself is the invariant.
        assert_eq!(
            (
                shell.state.read(cx).snapshot.graph.clone(),
                shell.state.read(cx).snapshot.phase,
            ),
            before
        );
        assert_eq!(
            shell.state.read(cx).snapshot.phase,
            crate::service::SessionPhase::Idle
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .audio_settings
                .output_overrides["output-a"]
                .sample_rate,
            Some(96_000)
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .audio_settings
                .output_overrides["output-a"]
                .format,
            moiren_engine::audio_config::SampleFormat::Pcm24
        );
        assert!(
            shell
                .state
                .read(cx)
                .snapshot
                .audio_settings
                .input_overrides
                .is_empty()
        );
        // Dirty device A, then save a global default and an unrelated override.
        super::settings::device_stream_mut(
            &mut shell.settings.editor.device.audio,
            Direction::Output,
            "output-a",
        )
        .buffer_frames = Some(128);
        shell.settings.editor.global = shell.current_settings_draft(cx);
        shell.settings.editor.global.audio.output.sample_rate = Some(88_200);
        super::settings::device_stream_mut(
            &mut shell.settings.editor.global.audio,
            Direction::Input,
            "input-b",
        )
        .buffer_frames = Some(512);
        shell.save_audio_settings(Editor::Global, false, cx);
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        shell.save_audio_settings(Editor::Device, false, cx)
    });
    settle_settings(&view, cx);
    let saved = crate::project::load_preferences_at(&path).unwrap();
    assert_eq!(saved.audio.output.sample_rate, Some(88_200));
    assert_eq!(
        saved.audio.output_overrides["output-a"].buffer_frames,
        Some(128)
    );
    assert_eq!(
        saved.audio.input_overrides["input-b"].buffer_frames,
        Some(512)
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("device-use-defaults", cx);
        window.click("device-settings-save", cx);
    });
    settle_settings(&view, cx);
    let saved = crate::project::load_preferences_at(&path).unwrap();
    assert!(!saved.audio.output_overrides.contains_key("output-a"));
    assert_eq!(saved.audio.effective_output("output-a"), saved.audio.output);
    assert!(saved.audio.input_overrides.contains_key("input-b"));
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

struct SettingsConfirmationDriver {
    log: std::sync::Arc<std::sync::Mutex<Vec<crate::service::AppRequest>>>,
    confirm: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
    initialized: bool,
    awaiting: bool,
}

impl crate::service::ServiceDriver for SettingsConfirmationDriver {
    fn request(
        &mut self,
        context: &mut crate::service::ServiceContext,
        _: &mut crate::service::JobPool,
        request: crate::service::AppRequest,
    ) {
        use crate::service::AppRequest;
        self.log.lock().unwrap().push(request.clone());
        match request {
            AppRequest::SetAudioSettings(settings) => {
                context.set_audio_settings(settings);
            }
            AppRequest::SetLatencyProfile(profile) => context.latency_profile = profile,
            AppRequest::ApplyGraph => {
                if self.fail.load(std::sync::atomic::Ordering::Acquire) {
                    context
                        .graph
                        .record_error("Device format rejected by test driver");
                } else {
                    let revision = context.graph.snapshot().candidate_revision;
                    context.graph.preparing(context.core.generation(), revision);
                    self.awaiting = true;
                }
            }
            _ => {}
        }
    }

    fn poll(
        &mut self,
        context: &mut crate::service::ServiceContext,
        _: &mut crate::service::JobPool,
        _: crate::service::RoundBudget,
    ) {
        if !self.initialized {
            self.initialized = true;
            let spec = crate::service::SessionSpec {
                source: crate::service::InputSelection::Tone { frequency_hz: 440. },
                output_endpoint_id: "fixture-output".into(),
                gain: 0.,
                pan: 0.,
                max_block_frames: 256,
            };
            context.graph.configure_session(&spec).unwrap();
            let generation = context.core.start(spec).unwrap();
            assert!(context.core.activated(generation));
            assert!(context.core.owner_started(generation));
            let revision = context.graph.snapshot().candidate_revision;
            context.graph.preparing(generation, revision);
            context.graph.running(generation, revision);
            context.active_audio_settings = Some(context.audio_settings.clone());
            context.active_latency_profile = Some(context.latency_profile);
        }
        if self.awaiting && self.confirm.load(std::sync::atomic::Ordering::Acquire) {
            self.awaiting = false;
            let revision = context.graph.snapshot().candidate_revision;
            context.graph.running(context.core.generation(), revision);
            context.active_audio_settings = Some(context.audio_settings.clone());
            context.active_latency_profile = Some(context.latency_profile);
        }
    }

    fn begin_exit(
        &mut self,
        context: &mut crate::service::ServiceContext,
        _: &mut crate::service::JobPool,
    ) {
        if let Some(generation) = context.core.owner_generation() {
            context.core.owner_reaped(generation);
        }
    }

    fn is_drained(&self) -> bool {
        true
    }
}

#[gpui_kit::test]
fn running_settings_save_and_restart_wait_for_confirmation_and_keep_failed_draft(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{AppRequest, AudioLatencyProfile};
    use crate::ui::pages::settings::DraftFeedback;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    cx.update(gpui_kit::init);
    let log = Arc::new(Mutex::new(Vec::new()));
    let confirm = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(AtomicBool::new(false));
    let runtime = ServiceRuntime::spawn_with_driver(SettingsConfirmationDriver {
        log: log.clone(),
        confirm: confirm.clone(),
        fail: fail.clone(),
        initialized: false,
        awaiting: false,
    })
    .unwrap();
    let handle = runtime.handle();
    let snapshot = wait_for(Duration::from_secs(2), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == crate::service::SessionPhase::Running)
    });
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = shell_with_state(cx, state);
    let path = settings_test_path("restart");
    view.update(cx, |shell, _| shell.preferences.path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        window.click("processing-rate-3", cx);
        window.render_frame(cx);
    });
    scroll_settings_to_bottom(cx);
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    );
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_audio_settings
                .as_ref()
                .unwrap()
                .processing
                .sample_rate,
            48_000
        );
        shell.settings.editor.global.profile = AudioLatencyProfile::Balanced;
    });
    cx.update(|window, cx| window.click("processing-rate-0", cx));
    scroll_settings_to_bottom(cx);
    cx.update(|window, cx| window.click("settings-restart", cx));
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        cx.update(|window, cx| window.render_frame(cx));
        log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    });
    let requests = log.lock().unwrap().clone();
    assert!(
        matches!(&requests[requests.len()-3], AppRequest::SetAudioSettings(settings) if settings.processing.sample_rate == 44_100)
    );
    assert_eq!(
        requests[requests.len() - 2],
        AppRequest::SetLatencyProfile(AudioLatencyProfile::Balanced)
    );
    assert_eq!(requests[requests.len() - 1], AppRequest::ApplyGraph);
    view.update(cx, |shell, cx| {
        assert!(shell.settings.editor.global.pending.is_some());
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_audio_settings
                .as_ref()
                .unwrap()
                .processing
                .sample_rate,
            48_000
        );
        assert_eq!(
            shell.preferences.values.audio.processing.sample_rate,
            96_000
        );
    });
    confirm.store(true, Ordering::Release);
    settle_settings(&view, cx);
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .processing
            .sample_rate,
        44_100
    );
    fail.store(true, Ordering::Release);
    cx.update(|window, cx| window.click("processing-rate-3", cx));
    scroll_settings_to_bottom(cx);
    cx.update(|window, cx| window.click("settings-restart", cx));
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        assert!(shell.settings.editor.global.dirty());
        assert_eq!(
            shell.settings.editor.global.audio.processing.sample_rate,
            96_000
        );
        assert!(matches!(
            shell.settings.editor.global.feedback,
            Some(DraftFeedback::Diagnostic { ref cause })
                if cause.contains("Device format rejected")
        ));
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_audio_settings
                .as_ref()
                .unwrap()
                .processing
                .sample_rate,
            44_100
        );
        assert_eq!(
            shell.preferences.values.audio.processing.sample_rate,
            44_100
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .processing
            .sample_rate,
        44_100
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn undo_redo_requests_round_trip_candidate_content(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{AppRequest, GraphEdit};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    let baseline = view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: moiren_core::graph::NodeKind::Eq,
                channels: 2,
            }));
        });
        shell.state.read(cx).snapshot.graph.nodes.len()
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == baseline + 1
                && shell.state.read(cx).snapshot.graph.can_undo
        })
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::UndoGraph);
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == baseline
        })
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::RedoGraph);
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == baseline + 1
        })
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn ctrl_z_keystroke_steps_the_undo_journal(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{AppRequest, GraphEdit};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|_, cx| {
        cx.bind_keys([
            gpui_kit::KeyBinding::new("ctrl-z", super::actions::Undo, None),
            gpui_kit::KeyBinding::new("ctrl-y", super::actions::Redo, None),
        ]);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: moiren_core::graph::NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == 5
        })
    });
    // The canvas key handler owns focus on the graph page; the keystroke
    // bubbles to the shell's undo action.
    cx.simulate_keystrokes("ctrl-z");
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == 4
        })
    });
    cx.simulate_keystrokes("ctrl-y");
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == 5
        })
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn ctrl_z_in_a_mixer_send_editor_leaves_the_graph_journal_alone(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|_, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "ctrl-z",
            super::actions::Undo,
            None,
        )]);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let handle = view.update(cx, |shell, cx| {
        use gpui_kit::Focusable;
        let edge = shell.state.read(cx).snapshot.graph.graph.edges()[0].id();
        shell.mixer.presentation.send_inputs[&edge]
            .read(cx)
            .focus_handle(cx)
    });
    cx.update(|window, cx| window.focus(&handle, cx));
    // The send-gain editor is a text field: typing_focus must claim it so
    // the shell's undo action declines to step the journal mid-edit.
    view.update(cx, |shell, cx| {
        assert!(shell.typing_focus(&handle, cx));
    });
    let nodes = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.nodes.len()
    });
    cx.simulate_keystrokes("ctrl-z");
    // Give a (wrongly) dispatched undo time to reach the service; the
    // journal must still be where it was.
    std::thread::sleep(Duration::from_millis(400));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
        assert_eq!(shell.state.read(cx).snapshot.graph.nodes.len(), nodes);
    });
    finish_service(runtime);
}

/// The inspector's click-to-type editors open focused; every root render on
/// the graph page then runs `keep_graph_focus`, which must treat them as
/// typing focus. A family missing from `typing_focus` loses focus on the
/// very next frame, the blur commits the prefilled text, and the editor
/// flashes open and closed (audit SD-2, failure mode B). The balance label
/// editor rides one of the families this fix added.
#[gpui_kit::test]
fn graph_inspector_value_editor_keeps_focus_across_root_renders(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    // Headless windows start inactive; without activation the focus loss
    // would never dispatch the real Blur that commits and closes the editor.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let gain = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Gain)
            .expect("fixed session graph holds a gain node")
            .id()
    });
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Graph);
        shell.chrome.inspector_open = true;
        shell.selection.node = Some(gain);
        cx.notify();
    });
    settle_panels(cx);
    cx.update(|window, cx| {
        window.click(
            gpui_kit::SharedString::from(format!("balance-label-{gain:?}")),
            cx,
        );
        window.render_frame(cx);
        window.render_frame(cx);
    });
    let input = view.update(cx, |shell, _| {
        shell.mixer.presentation.balance_inputs[&gain].clone()
    });
    view.update(cx, |shell, _| {
        assert_eq!(
            shell.mixer.presentation.editing_balance,
            Some(gain),
            "the editor must stay open instead of blur-committing the prefill"
        );
    });
    cx.update(|window, cx| {
        use gpui_kit::Focusable;
        let handle = input.read(cx).focus_handle(cx);
        assert!(
            window.focused(cx).is_some_and(|focused| focused == handle),
            "keep_graph_focus must leave a typing-focused inspector editor alone"
        );
    });
    finish_service(runtime);
}

/// The undo gate must cover the inspector's click-to-type families, not just
/// the older send/compressor editors: Ctrl+Z while one of them is focused
/// belongs to the text being typed, not to the graph journal (audit SD-2,
/// failure mode A).
#[gpui_kit::test]
fn ctrl_z_in_an_inspector_value_editor_leaves_the_graph_journal_alone(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{AppRequest, GraphEdit};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|_, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "ctrl-z",
            super::actions::Undo,
            None,
        )]);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    // Journal history so a (wrongly) dispatched undo has something to step.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: moiren_core::graph::NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.can_undo
        })
    });
    let gain = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Gain)
            .expect("fixed session graph holds a gain node")
            .id()
    });
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Graph);
        shell.chrome.inspector_open = true;
        shell.selection.node = Some(gain);
        cx.notify();
    });
    settle_panels(cx);
    cx.update(|window, cx| {
        window.click(
            gpui_kit::SharedString::from(format!("balance-label-{gain:?}")),
            cx,
        );
        window.render_frame(cx);
    });
    let handle = view.update(cx, |shell, cx| {
        use gpui_kit::Focusable;
        let handle = shell.mixer.presentation.balance_inputs[&gain]
            .read(cx)
            .focus_handle(cx);
        assert_eq!(shell.mixer.presentation.editing_balance, Some(gain));
        handle
    });
    // The inspector value editor is a text field: typing_focus must claim it
    // so the shell's undo action declines to step the journal mid-edit.
    view.update(cx, |shell, cx| {
        assert!(shell.typing_focus(&handle, cx));
    });
    let nodes = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.nodes.len()
    });
    cx.simulate_keystrokes("ctrl-z");
    // Give a (wrongly) dispatched undo time to reach the service; the
    // journal must still be where it was.
    std::thread::sleep(Duration::from_millis(400));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(
            graph.nodes.len(),
            nodes,
            "ctrl-z in the editor must not undo the graph"
        );
        assert!(graph.can_undo, "the journal step must still be pending");
    });
    finish_service(runtime);
}

/// Inside the cached inspector panel, a keystroke into a nested editor only
/// updates the editor's `InputState` — nothing notifies the shell or changes
/// the panel key. The nested observer pool is the only path that can carry
/// the keystroke to the screen; a family missing from it blind-types (audit
/// SD-1).
#[gpui_kit::test]
fn inspector_panel_rebuilds_when_a_nested_editor_moves(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    let gain = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Gain)
            .expect("fixed session graph holds a gain node")
            .id()
    });
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Graph);
        shell.chrome.inspector_open = true;
        shell.selection.node = Some(gain);
        cx.notify();
    });
    settle_panels(cx);
    // Opening the editor changes the panel key; the second frame flushes that
    // key-change rebuild so the baseline counts only nested-driven rebuilds.
    cx.update(|window, cx| {
        window.click(
            gpui_kit::SharedString::from(format!("balance-label-{gain:?}")),
            cx,
        );
        window.render_frame(cx);
        window.render_frame(cx);
    });
    let input = view.update(cx, |shell, _| {
        assert_eq!(
            shell.mixer.presentation.editing_balance,
            Some(gain),
            "the editor must be open for the keystroke to be nested"
        );
        shell.mixer.presentation.balance_inputs[&gain].clone()
    });
    let before = view.read_with(cx, |shell, _| shell.trace.borrow().inspector_renders);
    // One keystroke's worth of editor movement: notifies the InputState and
    // nothing else. With the nested pool wired the panel rebuilds on the
    // gate's leading edge; without it the character never reaches the screen.
    cx.update(|window, cx| {
        input.update(cx, |input, cx| input.set_value("L 10", window, cx));
        window.render_frame(cx);
    });
    let after = view.read_with(cx, |shell, _| shell.trace.borrow().inspector_renders);
    // `set_value` moves value and cursor in separate model notifications, so
    // the exact pass count rides the gate; the regression is the flat counter
    // of an unobserved family, so "at least one rebuild" is the invariant.
    assert!(
        after > before,
        "the nested editor notification must rebuild the cached inspector"
    );
    finish_service(runtime);
}

/// The inspector's meter pipeline must stay wired together (audit HY-10,
/// design 17 hazard 9): the inspector's canvases register their tracks into
/// the frame the sidebar hot surface reads, and that layer paints every one.
/// A second `SharedFrame` of the same type at the assembly point compiles,
/// runs, and paints nothing forever with no runtime way to notice — the
/// failure that muted the flyout — so the surface itself must census what it
/// sees and what it paints.
#[gpui_kit::test]
fn inspector_hot_layer_paints_the_registered_meter_tracks(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    let gain = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Gain)
            .expect("fixed session graph holds a gain node")
            .id()
    });
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = pages::WorkspaceView::Page(pages::Page::Graph);
        shell.chrome.inspector_open = true;
        shell.selection.node = Some(gain);
        cx.notify();
    });
    settle_panels(cx);
    // The selection's key change rebuilt the cached panel: `on_rebuild` reset
    // the frame and the fresh subtree's canvases re-registered their tracks;
    // the live hot layer paints them on every rendered frame after that.
    let (registered, painted) = view.update(cx, |shell, cx| {
        let meters = shell.inspector.meter_layer.read(cx).surface();
        let meters = meters.borrow();
        (meters.registered_tracks(), meters.painted_tracks())
    });
    assert!(
        registered > 0,
        "the hot surface must see the inspector's registered meter tracks"
    );
    assert!(
        painted >= registered,
        "the hot layer must paint every registered track"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_page_readout_editors_keep_focus_and_their_undo(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{AppRequest, GraphEdit};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|_, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "ctrl-z",
            super::actions::Undo,
            None,
        )]);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
    });
    // Give the journal one undoable edit so a leaked ctrl-z would show up
    // as a node count change.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: moiren_core::graph::NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == 5
        })
    });
    // The shell boots on the graph page, where keep_graph_focus hands
    // keystrokes back to the canvas whenever no known text field owns
    // focus. EVERY mixer editor family (send gains and the strip/inspector
    // readouts: fader, balance, node gain, node pan) must count as a text
    // field: otherwise the canvas steals focus on the next render and
    // ctrl-z steps the journal mid-edit.
    let handles = view.update(cx, |shell, cx| {
        use gpui_kit::Focusable;
        shell
            .mixer
            .presentation
            .text_input_entities()
            .map(|input| input.read(cx).focus_handle(cx))
            .collect::<Vec<_>>()
    });
    assert!(
        handles.len() >= 5,
        "expected send editors plus the four readout families, got {}",
        handles.len()
    );
    for handle in &handles {
        cx.update(|window, cx| window.focus(handle, cx));
        cx.update(|window, cx| window.render_frame(cx));
        view.update(cx, |shell, cx| {
            assert!(shell.typing_focus(handle, cx));
        });
    }
    cx.simulate_keystrokes("ctrl-z");
    // Give a (wrongly) dispatched undo time to reach the service; the
    // journal must still be where it was.
    std::thread::sleep(Duration::from_millis(400));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
        assert_eq!(shell.state.read(cx).snapshot.graph.nodes.len(), 5);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn inspector_panel_fills_the_column_even_for_its_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
        // Nothing is selected: the graph inspector shows its empty state.
        view.update(cx, |shell, _| {
            assert!(shell.selection.node.is_none() && shell.selection.route.is_none());
        });
        assert!(window.find("inspector-empty").visible());
        // The placeholder illustration rides in a soft tile above the label.
        let art = window.find("inspector-empty-art").bounds();
        assert!(art.size.width > px(56.) && art.size.height > px(56.));
        // Regression: the empty state's panel used to be as short as its
        // label, leaving the rest of the sidebar column transparent so the
        // inspector merged into the workspace background.
        let panel = window.find("graph-inspector").bounds();
        let column = window.find("graph-page").bounds();
        assert_eq!(
            f32::from(panel.size.height),
            f32::from(column.size.height),
            "the panel must fill the sidebar column"
        );
        // The illustration sits roughly centered in the column (the heading
        // above it shifts the optical center up slightly), not at the top.
        let art_center = f32::from(art.origin.y + art.size.height / 2.);
        let panel_center = f32::from(panel.origin.y + panel.size.height / 2.);
        assert!(
            (art_center - panel_center).abs() < 24.,
            "empty state centers vertically ({art_center} vs {panel_center})"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn statusbar_lists_its_items_and_opens_no_panel_at_rest(cx: &mut gpui_kit::TestAppContext) {
    // Reaching Running through the real driver needs live audio devices, so
    // the running-path projections are covered by the pure unit tests.
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        for id in [
            "statusbar-phase",
            "statusbar-clock",
            "statusbar-xrun",
            "statusbar-perf",
            "statusbar-buffer",
            "statusbar-format",
        ] {
            assert!(window.find(id).visible(), "{id} is on the bar while idle");
        }
        for panel in [
            "status-panel-engine",
            "status-panel-clock",
            "status-panel-xrun",
            "status-panel-perf",
            "status-panel-format",
        ] {
            assert!(
                window.try_find(panel).is_none(),
                "{panel} opens only on hover or click"
            );
        }
    });
    assert_eq!(
        view.read_with(cx, |shell, _| shell.chrome.status_popup),
        None
    );
    finish_service(runtime);
}

/// The engine's start/stop control lives in the status bar's engine panel;
/// the toolbar keeps only navigation, settings and the window controls.
#[gpui_kit::test]
fn the_toolbar_carries_no_transport_button(cx: &mut gpui_kit::TestAppContext) {
    let (_view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        assert!(window.try_find("transport").is_none());
        assert!(window.find("open-settings").visible());
        for id in ["titlebar-drag-leading", "titlebar-drag-trailing"] {
            assert!(window.find(id).bounds().size.width >= px(20.0));
        }
    });
    finish_service(runtime);
}

/// The status bar's hover state machine: a cold hover opens after the
/// intent delay, a second item switches at once, resting on the panel keeps
/// it open past the close grace (the deferred panel's enter arrives before
/// the item's leave, so the arbitration must not depend on event order),
/// and leaving both closes after the grace.
#[gpui_kit::test]
fn hovering_status_items_opens_switches_holds_and_closes_their_panels(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::ui::shell::status::StatusItem;
    let (view, cx, runtime) = test_shell(cx);
    let open = |cx: &mut gpui_kit::VisualTestContext| {
        view.read_with(cx, |shell, _| shell.chrome.status_popup)
    };
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        window.hover("statusbar-phase", cx);
    });
    assert_eq!(open(cx), None, "a cold hover waits out the intent delay");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(open(cx), Some(StatusItem::Engine));
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("status-panel-engine").visible());
        window.hover("statusbar-format", cx);
    });
    assert_eq!(
        open(cx),
        Some(StatusItem::Format),
        "browsing to another item switches without a delay"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("status-panel-engine").is_none());
        assert!(window.find("status-panel-format").visible());
        // The pointer travels up into the panel.
        window.hover("status-panel-format", cx);
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(
        open(cx),
        Some(StatusItem::Format),
        "resting on the panel keeps it open"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover("tab-graph", cx);
    });
    assert_eq!(
        open(cx),
        Some(StatusItem::Format),
        "the grace period has not elapsed yet"
    );
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(open(cx), None, "leaving both anchor and panel closes it");
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("status-panel-format").is_none());
    });
    finish_service(runtime);
}

/// Click is the touch path: it toggles the item's panel, and a press that
/// reaches the shell root (anything outside the items and panels) closes it.
#[gpui_kit::test]
fn clicking_a_status_item_toggles_its_panel_and_an_outside_press_closes_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::ui::shell::status::StatusItem;
    use gpui_kit::{Modifiers, MouseButton};
    let (view, cx, runtime) = test_shell(cx);
    let open = |cx: &mut gpui_kit::VisualTestContext| {
        view.read_with(cx, |shell, _| shell.chrome.status_popup)
    };
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        window.click("statusbar-xrun", cx);
        window.render_frame(cx);
        assert!(window.find("status-panel-xrun").visible());
    });
    assert_eq!(open(cx), Some(StatusItem::Xrun));
    cx.update(|window, cx| {
        window.click("statusbar-xrun", cx);
        window.render_frame(cx);
        assert!(window.try_find("status-panel-xrun").is_none());
    });
    assert_eq!(open(cx), None, "a second click closes the panel");
    // Reopen, then press the bar's empty middle: no item claims it, so the
    // shell root's outside-press listener sees it.
    let outside = cx.update(|window, cx| {
        window.click("statusbar-xrun", cx);
        window.render_frame(cx);
        window.find("engine-statusbar").bounds().center()
    });
    assert_eq!(open(cx), Some(StatusItem::Xrun));
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    assert_eq!(open(cx), None, "an outside press dismisses the panel");
    finish_service(runtime);
}

/// Every panel hangs above the bar by the shared popover gap and stays
/// inside the window, at the comfortable width and below the compact
/// breakpoint alike.
#[gpui_kit::test]
fn status_panels_hang_above_the_bar_inside_the_window(cx: &mut gpui_kit::TestAppContext) {
    use crate::ui::shell::status::StatusItem;
    let (view, cx, runtime) = test_shell(cx);
    for (width, height) in [(1440.0, 900.0), (1024.0, 720.0)] {
        cx.simulate_resize(gpui_kit::size(px(width), px(height)));
        for item in [
            StatusItem::Engine,
            StatusItem::Clock,
            StatusItem::Xrun,
            StatusItem::Perf,
            StatusItem::Format,
        ] {
            view.update(cx, |shell, cx| {
                shell.chrome.status_popup = Some(item);
                cx.notify();
            });
            cx.update(|window, cx| {
                window.render_frame(cx);
                let bar = window.find("engine-statusbar").bounds();
                let panel = window.find(item.panel_id()).bounds();
                let gap = f32::from(bar.top() - panel.bottom());
                assert!(
                    (gap - theme::POPOVER_ANCHOR_GAP).abs() < 0.5,
                    "{item:?} hangs {gap}px above the bar at {width}x{height}"
                );
                assert!(
                    panel.left() >= px(0.) && panel.right() <= px(width) && panel.top() >= px(0.),
                    "{item:?} stays inside the {width}x{height} window: {panel:?}"
                );
            });
        }
    }
    finish_service(runtime);
}

/// The XRUN panel's reset is a control-side baseline move: history and the
/// headline count clear at once, with no service request behind it.
#[gpui_kit::test]
fn the_xrun_panel_reset_clears_the_statistics_window(cx: &mut gpui_kit::TestAppContext) {
    use crate::ui::monitoring::runtime::StreamCounters;
    use crate::ui::shell::status::StatusItem;
    let (view, cx, runtime) = test_shell(cx);
    let node = moiren_core::graph::LogicalGraph::new()
        .create_node(moiren_core::graph::NodeKind::Sink, 2)
        .unwrap();
    let total = |cx: &mut gpui_kit::VisualTestContext| {
        view.read_with(cx, |shell, cx| shell.state.read(cx).runtime.xruns.total())
    };
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let now = std::time::Instant::now();
            let counters = |xruns: u64| StreamCounters {
                node,
                xruns,
                native_xruns: 0,
                engine_blocks: xruns * 1_000,
            };
            state.runtime.xruns.ingest(1, &[counters(0)], now);
            state.runtime.xruns.ingest(1, &[counters(3)], now);
        });
        shell.chrome.status_popup = Some(StatusItem::Xrun);
        cx.notify();
    });
    assert_eq!(total(cx), 3);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        window.click("status-xrun-reset", cx);
    });
    assert_eq!(total(cx), 0, "reset clears the statistics window");
    finish_service(runtime);
}

/// Start is start-by-graph (design 23 §4.2): the engine panel's button
/// dispatches from idle without a UI-side verdict on the graph, and only an
/// in-flight request or a transitional phase disables it.
#[gpui_kit::test]
fn the_engine_panel_button_dispatches_the_transport_toggle(cx: &mut gpui_kit::TestAppContext) {
    use crate::ui::shell::status::StatusItem;
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.chrome.status_popup = Some(StatusItem::Engine);
        cx.notify();
    });
    assert!(!view.read_with(cx, |shell, cx| shell.state.read(cx).pending_request));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        window.click("status-engine-toggle", cx);
    });
    assert!(
        view.read_with(cx, |shell, cx| shell.state.read(cx).pending_request),
        "the click dispatched the start request"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn expanded_library_bounds_its_scroll_area_and_scrolls_to_lower_templates(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{Bounds, WindowBounds, WindowOptions};
    cx.update(gpui_kit::init);
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let page = cx.new(|_| super::PageCell::default());
    // Wide enough for the expanded sidebar, short enough that the template
    // list must scroll. Test windows open maximized by default, so the short
    // bounds have to be requested at open time.
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    gpui_kit::size(px(1440.), px(480.)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut shell = super::StudioShell::new(state, window, cx, None);
                    shell.test_page = Some(page);
                    shell.preferences.path = None;
                    shell.autosave.path = None;
                    shell
                })
            },
        )
        .unwrap()
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    super::settle_panels(&mut visual);
    let probe = cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("session-io-panel").visible(),
            "the expanded sidebar is mounted"
        );
        let panel = window.find("session-io-panel").bounds();
        let scroll = window.find("node-library-scroll").bounds();
        assert!(
            f32::from(scroll.size.height) < f32::from(panel.size.height),
            "the list is bounded by the panel, not by its own content"
        );
        assert!(
            f32::from(panel.size.height) <= 480.,
            "the panel cannot overflow the window height ({}px)",
            f32::from(panel.size.height)
        );
        assert!(
            !window.find("library-Recorder").visible(),
            "the last template starts below the fold"
        );
        window.scroll(
            "node-library-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-800.))),
            cx,
        );
        window.render_frame(cx);
        assert!(
            window.find("library-Recorder").visible(),
            "scrolling reaches the lower templates for dragging"
        );
    });
    assert!(probe.is_ok(), "the probe window stays open");
    finish_service(runtime);
}

/// Opening, holding and closing a status panel must not do more strip-panel
/// work than an idle rendered frame (design 17): the bar is a live sibling
/// of the pages, its open state rides no key, and the panel's fade runs on
/// the live surface. The panel opens and closes through seeded shell state
/// — the capture harness's pattern — because a pointer move itself dirties
/// strip paint (verified identical on origin/main). One rendered frame
/// rebuilds a few strips on the current build — the same on main — so the
/// invariant is pinned relative to that idle-frame floor, not against zero.
#[gpui_kit::test]
fn opening_a_status_panel_rebuilds_no_cached_page_panel(cx: &mut gpui_kit::TestAppContext) {
    use crate::ui::shell::status::StatusItem;
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.0), px(900.0)));
        window.render_frame(cx);
        window.click("tab-mixer", cx);
    });
    settle_panels(cx);
    let strips = |cx: &mut gpui_kit::VisualTestContext| {
        view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders)
    };
    // The idle-frame floor: one rendered frame's strip work with no panel.
    let before = strips(cx);
    cx.update(|window, cx| window.render_frame(cx));
    let floor = strips(cx) - before;
    // Open, hold over its fade, close: every step may only cost what an
    // idle frame costs, never a page-panel rebuild on top.
    view.update(cx, |shell, cx| {
        shell.chrome.status_popup = Some(StatusItem::Perf);
        cx.notify();
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("status-panel-perf").visible());
        // Frames while the panel is open, fade included: simulate_next_frame
        // pairs draw without the refresh that carries the idle noise.
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        assert!(window.find("status-panel-perf").visible());
    });
    let open = strips(cx);
    assert!(
        open - before <= 2 * floor,
        "an open, animating status panel must not exceed two idle frames of          strip work (open delta {} over floor {})",
        open - before,
        floor
    );
    view.update(cx, |shell, cx| shell.status_close(cx));
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("status-panel-perf").is_none());
    });
    let closed = strips(cx);
    assert!(
        closed - open <= floor,
        "closing the panel must not exceed one idle frame of strip work          (close delta {} over floor {})",
        closed - open,
        floor
    );
    finish_service(runtime);
}

/// The status bar's error summary is the unlocated errors' only surface
/// since the error strip: absent while clean, a located fault never mounts
/// it (that fault belongs to its own strip/card), a dispatch failure alone
/// — written synchronously with no publish behind it, the regression the
/// old error-strip key carried — mounts it, and clicking opens the
/// graph-wide fault popover.
#[gpui_kit::test]
fn statusbar_summarizes_unlocated_errors_and_opens_the_fault_popover(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{GraphFault, GraphSubject};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("statusbar-faults").is_none(),
            "a clean session wears no summary"
        );
    });

    // A located fault goes to its node, never to the status bar.
    let target = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let mut snapshot = (*state.snapshot).clone();
            let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
            graph.faults.push(GraphFault {
                subject: GraphSubject::Node(target),
                cause: "The input source configuration is invalid".into(),
                hint: None,
            });
            state.snapshot = std::sync::Arc::new(snapshot);
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        assert!(
            window.try_find("statusbar-faults").is_none(),
            "a located fault must not surface on the status bar"
        );
    });

    // A graph-wide fault does.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let mut snapshot = (*state.snapshot).clone();
            let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
            graph.faults.push(GraphFault {
                subject: GraphSubject::Graph,
                cause: "Resumed without one node because its device is offline".into(),
                hint: None,
            });
            state.snapshot = std::sync::Arc::new(snapshot);
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("statusbar-faults").is_some());
    });

    // A dispatch failure with no publish behind it must mount the summary
    // on its own notification.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch_error = Some(crate::ui::state::UiFailure::Dispatch(
                crate::service::DispatchError::InvalidConfig,
            ));
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        assert!(
            window.try_find("statusbar-faults").is_some(),
            "dispatch_error alone must surface the summary"
        );
    });

    // The click opens the graph-wide fault popover.
    cx.update(|window, cx| {
        window.click("statusbar-faults", cx);
        window.draw(cx).clear(cx);
    });
    assert!(
        view.read_with(cx, |shell, _| shell
            .chrome
            .fault_popover
            .is_some_and(|popover| popover.node.is_none())),
        "the summary click opens the graph-wide fault popover"
    );
    cx.update(|window, _| {
        assert!(window.try_find("fault-popover").is_some());
    });
    finish_service(runtime);
}
