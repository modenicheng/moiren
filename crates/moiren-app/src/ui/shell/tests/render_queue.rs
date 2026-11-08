//! Render requests use the existing stopped-save/explicit-restart transaction.
use super::*;

fn reveal(cx: &mut gpui_kit::VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        for _ in 0..35 {
            let item = window.find(id.to_owned());
            let viewport = window.find("settings-scroll").bounds();
            if item.visible() && viewport.contains(&item.bounds().center()) {
                return;
            }
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-180.))),
                cx,
            );
        }
        panic!("render control is not visible: {id}");
    });
}

#[gpui_kit::test]
fn render_queue_stopped_save_preserves_independent_input_policies_and_defaults(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("render-queue");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-transport", cx);
    });
    reveal(cx, "render-policy-1");
    cx.update(|window, cx| window.click("render-policy-1", cx));
    reveal(cx, "render-target-3");
    cx.update(|window, cx| {
        window.click("render-target-3", cx); // 512 output-rate frames.
        window.click("settings-save", cx);
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.phase, crate::service::SessionPhase::Idle);
        assert!(snapshot.active_transport_latency.is_none());
        assert!(snapshot.transport_latency.process.is_none());
        assert!(snapshot.transport_latency.vad.is_none());
        assert_eq!(snapshot.audio_settings.output.buffer_frames, None);
        assert_eq!(
            snapshot.transport_latency.render.unwrap().target_frames,
            512
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .render
            .unwrap()
            .target_frames,
        512
    );
    cx.update(|window, cx| {
        window.click("render-policy-0", cx);
        window.click("settings-save", cx);
    });
    settle_settings(&view, cx);
    assert!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .render
            .is_none()
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn render_queue_running_restart_waits_for_matching_active_confirmation(
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
    let path = settings_test_path("render-queue-running");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-transport", cx);
    });
    reveal(cx, "render-policy-1");
    cx.update(|window, cx| window.click("render-policy-1", cx));
    reveal(cx, "render-target-3");
    cx.update(|window, cx| {
        window.click("render-target-3", cx);
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
        let snapshot = &shell.state.read(cx).snapshot;
        assert!(snapshot.active_transport_latency.unwrap().render.is_none());
        assert_eq!(
            snapshot.transport_latency.render.unwrap().target_frames,
            512
        );
    });

    reveal(cx, "render-target-2");
    cx.update(|window, cx| {
        window.click("render-target-2", cx); // 384 frames, not the saved 512.
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
        assert!(shell.settings.editor.global.pending.is_some());
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(
            snapshot.transport_latency.render.unwrap().target_frames,
            384
        );
        assert!(snapshot.active_transport_latency.unwrap().render.is_none());
        assert_eq!(
            shell
                .preferences
                .transport_latency
                .render
                .unwrap()
                .target_frames,
            512
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .render
            .unwrap()
            .target_frames,
        512
    );
    confirm.store(true, Ordering::Release);
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(
            snapshot
                .active_transport_latency
                .unwrap()
                .render
                .unwrap()
                .target_frames,
            384
        );
        assert!(snapshot.active_transport_latency.unwrap().process.is_none());
        assert!(snapshot.active_transport_latency.unwrap().vad.is_none());
        assert!(!shell.settings.editor.global.dirty());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .render
            .unwrap()
            .target_frames,
        384
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn render_only_change_warns_that_measured_input_needs_fresh_admission(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let snapshot = std::sync::Arc::make_mut(&mut state.snapshot);
            snapshot.measured_process = Some(crate::service::MeasuredProcessSummary {
                selection_id: 1,
                request: crate::service::MeasuredProcessRequest {
                    node: snapshot.graph.graph.nodes()[0].id(),
                    session_generation: crate::service::SessionGeneration(1),
                    native_owner_generation: 1,
                    graph_revision: 1,
                    capacity_frames: 16384,
                    target_frames: 1536,
                    jitter_frames: 96,
                },
                process_pid: 42,
                process_creation_time_100ns: 123,
                delivery: crate::service::measured_process::MeasuredDeliveryBound {
                    sample_rate: 48_000,
                    max_packet_frames: 480,
                    max_gap_frames: 531,
                },
                observed_window: None,
            });
        });
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-transport", cx);
    });
    reveal(cx, "render-policy-1");
    cx.update(|window, cx| {
        window.click("render-policy-1", cx);
        let warning = window.find("settings-measured-invalidation");
        assert!(warning.visible());
        let text = warning.label().unwrap();
        assert!(text.contains("render queue"), "{text}");
        assert!(text.contains("fresh admission"), "{text}");
    });
    finish_service(runtime);
}
