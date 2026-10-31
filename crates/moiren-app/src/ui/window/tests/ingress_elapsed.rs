//! Real headless controls and service confirmations, with no native owner.
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

fn click_ingress_choice(cx: &mut gpui_kit::VisualTestContext, index: usize) {
    cx.update(|window, cx| {
        let id = gpui_kit::SharedString::from(format!("process-ingress-elapsed-{index}"));
        for _ in 0..24 {
            let target = window.find(id.clone());
            let viewport = window.find("settings-scroll").bounds();
            if target.visible()
                && target.bounds().origin.y + target.bounds().size.height
                    < viewport.origin.y + viewport.size.height - px(12.)
            {
                break;
            }
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-180.))),
                cx,
            );
        }
        assert!(window.find(id.clone()).visible());
        assert!(window.try_find("process-expert-fields").is_none());
        window.click(id, cx);
    });
}

#[gpui_kit::test]
fn ingress_elapsed_click_stopped_save_persists_without_starting_audio(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("ingress-stopped");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-transport", cx);
    });
    click_ingress_choice(cx, 3); // 50 ms; Compatible queue policy is unchanged.
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .settings
                .global
                .transport
                .process_max_ingress_elapsed_100ns,
            Some(500_000)
        );
        assert!(shell.settings.global.transport.process.is_none());
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .transport_latency
                .process_max_ingress_elapsed_100ns,
            None
        );
        assert!(shell.settings.global.dirty());
    });
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.phase, crate::service::SessionPhase::Idle);
        assert!(snapshot.active_audio_settings.is_none());
        assert!(snapshot.active_transport_latency.is_none());
        assert_eq!(
            snapshot.transport_latency.process_max_ingress_elapsed_100ns,
            Some(500_000)
        );
        assert!(!shell.settings.global.dirty());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .process_max_ingress_elapsed_100ns,
        Some(500_000)
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn ingress_elapsed_click_restart_keeps_requested_and_active_distinct_until_confirmation(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{AppRequest, SessionPhase};
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
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let state = cx.new(|_| UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    let path = settings_test_path("ingress-running");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(900.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-transport", cx);
    });
    click_ingress_choice(cx, 1); // 10 ms.
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
            .process_max_ingress_elapsed_100ns,
        Some(100_000)
    );
    click_ingress_choice(cx, 2); // 20 ms, then explicitly restart.
    cx.update(|window, cx| window.click("settings-restart", cx));
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll())
        });
        cx.update(|window, cx| window.render_frame(cx));
        log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    });
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(
            snapshot.transport_latency.process_max_ingress_elapsed_100ns,
            Some(200_000)
        );
        assert_eq!(
            snapshot
                .active_transport_latency
                .unwrap()
                .process_max_ingress_elapsed_100ns,
            None
        );
        assert!(shell.settings.global.pending.is_some());
        assert_eq!(
            shell
                .preferences
                .transport_latency
                .process_max_ingress_elapsed_100ns,
            Some(100_000)
        );
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-feedback")
                .label()
                .unwrap()
                .contains("waiting for the active session"),
            "requested settings alone must not render a successful restart"
        );
        assert!(
            window
                .find("measured-process-requested")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: 200000 x 100 ns")
        );
        assert!(
            window
                .find("measured-process-active")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: Disabled")
        );
        window.click("settings-diagnostics", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("process-input-requested")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: 200000 x 100 ns")
        );
        assert!(
            window
                .find("process-input-active")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: Disabled")
        );
    });
    confirm.store(true, Ordering::Release);
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_transport_latency
                .unwrap()
                .process_max_ingress_elapsed_100ns,
            Some(200_000)
        );
        assert!(!shell.settings.global.dirty());
        assert!(shell.settings.global.pending.is_none());
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("process-input-active")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: 200000 x 100 ns")
        );
        window.click("settings-transport", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("measured-process-active")
                .label()
                .unwrap()
                .contains("host ingress elapsed limit: 200000 x 100 ns")
        );
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .process_max_ingress_elapsed_100ns,
        Some(200_000)
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
