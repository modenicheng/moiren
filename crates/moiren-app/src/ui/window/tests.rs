use super::{ServiceRuntime, StudioShell, UiState, pages, theme};
use crate::ui::window::test_harness::{
    finish_service, initial_projection, shell_with_state, spawn_service, test_shell, wait_for,
    wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, point, px};
use std::time::Duration;

mod ingress_elapsed;

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
            shell.presentation.page = pages::Page::Mixer;
            shell.camera.pan_x = 84.0;
            shell.sidebar_open = true;
            shell.inspector_open = true;
            shell.device_tab = super::devices::DeviceTab::Inputs;
            shell.selected_device = Some("selected-input".into());
            shell.node_positions.insert(test_node, (12.0, 34.0));
            shell.project_name = "Keep candidate".into();
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
            .update(cx, |shell, cx| {
                let test_node = state.read(cx).snapshot.graph.graph.nodes()[0].id();
                assert_eq!(shell.presentation.page, pages::Page::Mixer);
                assert_eq!(shell.camera.pan_x, 84.0);
                assert_eq!(shell.node_positions.get(&test_node), Some(&(12.0, 34.0)));
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
        shell.preferences = project::AppPreferences::default();
        shell.preferences_path = Some(path.clone());
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
    view.update(cx, |shell, cx| {
        shell.preferences.latency_profile = AudioLatencyProfile::Balanced;
        shell.save_preferences(cx);
        shell.preferences.latency_profile = AudioLatencyProfile::Low;
        shell.save_preferences(cx);
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
                crate::ui::theme::LIBRARY_RAIL_WIDTH
            } + shell.camera.pan_x
                + (wire.from.0 + wire.to.0) * 0.5 * shell.camera.zoom),
            px(shell.camera.pan_y + wire.from.1 * shell.camera.zoom),
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
        assert!(shell.selected_route.is_some());
        assert!(shell.selected_node.is_none());
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
        view.update(cx, |shell, _| !shell.settings.busy())
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn settings_modal_blocks_graph_input_and_restores_workspace_and_focus(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        shell.camera.pan_x = 123.;
        shell.camera.pan_y = 87.;
        shell.camera.zoom = 0.89;
        shell.camera.adjusted = true;
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[1].id());
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        assert!(window.find("settings-dialog").visible());
        assert!(
            window.find("graph-page").visible(),
            "workspace remains mounted"
        );
        assert!(
            window.find("settings-save").visible(),
            "footer stays visible"
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
                shell.settings.focus.contains_focused(window, cx),
                "Tab remains inside modal"
            );
            assert_eq!(shell.state.read(cx).snapshot.graph.graph.nodes().len(), 4);
            assert!(shell.selected_node.is_some());
            assert_eq!(
                (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
                (123., 87., 0.89)
            );
        });
        window.press("escape", cx);
        assert!(window.try_find("settings-dialog").is_none());
        view.update(cx, |shell, cx| {
            assert_eq!(shell.presentation.page, pages::Page::Graph);
            assert!(!shell.settings_open);
            assert!(!shell.settings.focus.contains_focused(window, cx));
        });
        window.click("open-settings", cx);
        window.click_at("settings-backdrop", point(px(8.), px(8.)), cx);
        assert!(window.try_find("settings-dialog").is_none());
        assert_eq!(window.find("tab-graph").selected(), Some(true));
        window.click("open-settings", cx);
        // A click at the underlying tab is a backdrop dismissal only.
        window.click("tab-mixer", cx);
        assert_eq!(window.find("tab-graph").selected(), Some(true));
        assert!(window.try_find("settings-dialog").is_none());
        window.click("open-settings", cx);
        window.click("close-settings", cx);
        assert!(window.try_find("settings-dialog").is_none());
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
        shell.preferences = crate::project::AppPreferences::default();
        shell.preferences_path = Some(path.clone());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        assert!(window.find("settings-dialog").visible());
        window.click("processing-rate-0", cx);
        window.render_frame(cx);
        window.click("processing-precision-1", cx);
        window.render_frame(cx);
        window.click("processing-buffer-6", cx);
        window.render_frame(cx);
        window.click("audio-backend-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("default-input-mode-0").is_none());
        assert!(window.try_find("default-input-format-0").is_none());
        assert!(window.try_find("settings-restart").is_none());
    });
    view.update(cx, |shell, cx| {
        shell.settings.global.transport.vad_recovery_cap_periods = Some(3);
        assert!(shell.settings.global.dirty());
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
        assert_eq!(shell.preferences.audio.processing.sample_rate, 48_000);
        assert!(!shell.state.read(cx).pending_request);
    });
    assert!(!path.exists());
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert_eq!(state.snapshot.phase, crate::service::SessionPhase::Idle);
        assert!(state.snapshot.active_audio_settings.is_none());
        assert!(state.snapshot.active_transport_latency.is_none());
        assert_eq!(
            state.snapshot.transport_latency.vad_recovery_cap_periods,
            Some(3)
        );
        assert_eq!(state.snapshot.audio_settings.backend, AudioBackend::Asio);
        assert_eq!(
            state.snapshot.audio_settings.processing.precision,
            ProcessingPrecision::F64
        );
        assert_eq!(state.snapshot.audio_settings.processing.block_frames, 4096);
        assert!(!shell.settings.global.dirty());
        assert_eq!(shell.project_message.as_deref(), Some("Preferences saved"));
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .processing
            .sample_rate,
        44_100
    );
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .vad_recovery_cap_periods,
        Some(3)
    );
    cx.update(|window, cx| {
        window.click("settings-appearance", cx);
        window.click("density-1", cx);
        window.click("settings-save", cx);
    });
    settle_settings(&view, cx);
    assert_eq!(
        crate::project::load_preferences_at(&path).unwrap().density,
        crate::project::UiDensity::Compact
    );
    finish_service(runtime);
    cx.update(|window, cx| {
        window.click("density-0", cx);
        window.click("settings-save", cx);
    });
    view.update(cx, |shell, _| {
        assert!(shell.settings.global.error.is_some());
        assert!(shell.settings.global.dirty());
        assert_eq!(shell.density, super::Density::Compact);
    });
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
        shell.preferences_path = Some(path.clone());
        shell.set_page(pages::Page::Devices, cx);
    });
    let before = view.update(cx, |shell, cx| {
        shell.state.read(cx).selection.output_id.clone()
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("inspect-output-a", cx);
        window.click("device-override", cx);
        window.click("device-output-rate-4", cx);
        window.click("device-output-format-2", cx);
        window.click("device-settings-save", cx);
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).selection.output_id, before);
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
            &mut shell.settings.device.audio,
            Direction::Output,
            "output-a",
        )
        .buffer_frames = Some(128);
        shell.settings.global = shell.current_settings_draft(cx);
        shell.settings.global.audio.output.sample_rate = Some(88_200);
        super::settings::device_stream_mut(
            &mut shell.settings.global.audio,
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
    fail_session: bool,
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
            AppRequest::SetTransportLatency(settings) => {
                context.set_transport_latency(settings);
            }
            AppRequest::SetLatencyProfile(profile) => context.latency_profile = profile,
            AppRequest::ApplyGraph => {
                if self.fail.load(std::sync::atomic::Ordering::Acquire) {
                    let error = "Device format rejected by test driver";
                    context.graph.record_error(error);
                    if self.fail_session {
                        let generation = context.core.generation();
                        context.core.failed(generation, error.into());
                        context.core.owner_reaped(generation);
                        context.active_audio_settings = None;
                        context.active_transport_latency = None;
                        context.active_latency_profile = None;
                    }
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
                limit: crate::service::RunLimit::UntilStopped,
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
            context.active_transport_latency = Some(context.transport_latency);
            context.active_latency_profile = Some(context.latency_profile);
        }
        if self.awaiting && self.confirm.load(std::sync::atomic::Ordering::Acquire) {
            self.awaiting = false;
            let revision = context.graph.snapshot().candidate_revision;
            context.graph.running(context.core.generation(), revision);
            context.active_audio_settings = Some(context.audio_settings.clone());
            context.active_transport_latency = Some(context.transport_latency);
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
        fail_session: false,
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
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        window.click("processing-rate-3", cx);
        window.render_frame(cx);
        window.click("settings-save", cx);
    });
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
        shell.settings.global.profile = AudioLatencyProfile::Balanced;
    });
    cx.update(|window, cx| {
        window.click("processing-rate-0", cx);
        window.click("settings-restart", cx);
    });
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
        assert!(shell.settings.global.pending.is_some());
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
        assert_eq!(shell.preferences.audio.processing.sample_rate, 96_000);
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
    cx.update(|window, cx| {
        window.click("processing-rate-3", cx);
        window.click("settings-restart", cx);
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        assert!(shell.settings.global.dirty());
        assert_eq!(shell.settings.global.audio.processing.sample_rate, 96_000);
        assert!(
            shell
                .settings
                .global
                .error
                .as_deref()
                .unwrap()
                .contains("Device format rejected")
        );
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
        assert_eq!(shell.preferences.audio.processing.sample_rate, 44_100);
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
fn vad_recovery_cap_click_save_and_restart_wait_for_active_confirmation(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::AppRequest;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    cx.update(gpui_kit::init);
    let log = Arc::new(Mutex::new(Vec::new()));
    let confirm = Arc::new(AtomicBool::new(false));
    let runtime = ServiceRuntime::spawn_with_driver(SettingsConfirmationDriver {
        log: log.clone(),
        confirm: confirm.clone(),
        fail: Arc::new(AtomicBool::new(false)),
        fail_session: false,
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
    let path = settings_test_path("vad-recovery-cap");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-transport", cx);
        for _ in 0..24 {
            let target = window.find("vad-recovery-cap-1");
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
        assert!(window.try_find("vad-expert-fields").is_none());
        window.click("vad-recovery-cap-1", cx);
    });
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell.settings.global.transport.vad_recovery_cap_periods,
            Some(1)
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .transport_latency
                .vad_recovery_cap_periods,
            None
        );
    });
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    );
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .vad_recovery_cap_periods,
        Some(1)
    );
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_transport_latency
                .unwrap()
                .vad_recovery_cap_periods,
            None
        );
    });
    cx.update(|window, cx| {
        window.click("vad-recovery-cap-2", cx);
        window.click("settings-restart", cx);
    });
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.poll();
            })
        });
        cx.update(|window, cx| window.render_frame(cx));
        log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    });
    view.update(cx, |shell, cx| {
        assert!(shell.settings.global.pending.is_some());
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .transport_latency
                .vad_recovery_cap_periods,
            Some(2)
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_transport_latency
                .unwrap()
                .vad_recovery_cap_periods,
            None
        );
        assert_eq!(
            shell.preferences.transport_latency.vad_recovery_cap_periods,
            Some(1)
        );
    });
    confirm.store(true, Ordering::Release);
    settle_settings(&view, cx);
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .vad_recovery_cap_periods,
        Some(2)
    );
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_transport_latency
                .unwrap()
                .vad_recovery_cap_periods,
            Some(2)
        );
        assert!(!shell.settings.global.dirty());
    });
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn rejected_transport_restart_keeps_draft_and_saved_preferences_unchanged(
    cx: &mut gpui_kit::TestAppContext,
) {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    cx.update(gpui_kit::init);
    let confirm = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(AtomicBool::new(true));
    let log = Arc::new(Mutex::new(Vec::new()));
    let runtime = ServiceRuntime::spawn_with_driver(SettingsConfirmationDriver {
        log: log.clone(),
        confirm: confirm.clone(),
        fail: fail.clone(),
        fail_session: true,
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
    let path = settings_test_path("transport-rejection");
    crate::project::save_preferences_at(&path, &crate::project::AppPreferences::default()).unwrap();
    view.update(cx, |shell, cx| {
        shell.preferences = crate::project::AppPreferences::default();
        shell.settings.global = shell.current_settings_draft(cx);
        shell.settings.device = shell.current_settings_draft(cx);
        shell.preferences_path = Some(path.clone());
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
    });

    view.update(cx, |shell, cx| {
        shell.settings.global.transport.process = Some(crate::service::ExpertInputBuffer {
            target_frames: 1024,
            ..Default::default()
        });
        shell.settings.global.transport.vad_recovery_cap_periods = Some(1);
        shell.save_audio_settings(super::settings::Editor::Global, true, cx);
        assert!(shell.settings.global.pending.is_some());
        assert!(shell.settings.global.pending.as_ref().unwrap().restart);
    });
    settle_settings(&view, cx);

    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, crate::service::AppRequest::ApplyGraph)),
        "restart request was not dispatched: {:?}",
        *log.lock().unwrap()
    );
    view.update(cx, |shell, cx| {
        let draft = &shell.settings.global;
        let state = shell.state.read(cx);
        assert!(draft.dirty());
        assert!(draft.transport.process.is_some());
        assert_eq!(draft.transport.vad_recovery_cap_periods, Some(1));
        assert!(draft.error.is_some());
        assert_eq!(state.snapshot.phase, crate::service::SessionPhase::Failed);
        assert_eq!(state.snapshot.active_transport_latency, None);
        assert_eq!(state.snapshot.transport_latency, draft.transport);
        assert_eq!(shell.preferences.transport_latency, Default::default());
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let feedback = window.find("settings-feedback");
        assert!(feedback.visible());
        assert!(feedback.label().unwrap().contains("Device format rejected"));
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency,
        Default::default()
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
            gpui_kit::KeyBinding::new("ctrl-z", super::app_view::Undo, None),
            gpui_kit::KeyBinding::new("ctrl-y", super::app_view::Redo, None),
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
            super::app_view::Undo,
            None,
        )]);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        shell.presentation.page = pages::Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let handle = view.update(cx, |shell, cx| {
        use gpui_kit::Focusable;
        let edge = shell.state.read(cx).snapshot.graph.graph.edges()[0].id();
        shell.mixer.send_inputs[&edge].read(cx).focus_handle(cx)
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

#[gpui_kit::test]
fn inspector_panel_fills_the_column_even_for_its_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.render_frame(cx);
        // Nothing is selected: the graph inspector shows its empty state.
        view.update(cx, |shell, _| {
            assert!(shell.selected_node.is_none() && shell.selected_route.is_none());
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
fn statusbar_shows_the_configured_format_and_hides_route_and_clock_when_idle(
    cx: &mut gpui_kit::TestAppContext,
) {
    // Reaching Running through the real driver needs live audio devices, so
    // the running-path composition is covered by chrome's unit tests.
    let (_view, cx, runtime) = test_shell(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("statusbar-format").visible(),
            "the configured processing format is informative even when idle"
        );
        assert!(window.try_find("statusbar-route").is_none());
        assert!(window.try_find("statusbar-drift").is_none());
    });
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
                    shell.preferences_path = None;
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
