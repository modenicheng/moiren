//! Exact expert requests traverse the normal transaction; no native audio runs.
use super::*;

fn reveal(cx: &mut gpui_kit::VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        for _ in 0..45 {
            let item = window.find(id.to_owned());
            let viewport = window.find("settings-scroll").bounds();
            if item.visible() && viewport.contains(&item.bounds().center()) {
                return;
            }
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(point(
                    px(0.),
                    if item.bounds().center().y < viewport.origin.y {
                        px(180.)
                    } else {
                        px(-180.)
                    },
                )),
                cx,
            );
        }
        panic!("expert control is not visible: {id}");
    });
}

fn open(cx: &mut gpui_kit::VisualTestContext) {
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.click("settings-nav-transport", cx);
    });
}

fn text(cx: &mut gpui_kit::VisualTestContext, id: &str, value: &str) {
    reveal(cx, id);
    cx.update(|window, cx| {
        window.click(id.to_owned(), cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(value, cx);
    });
}

#[gpui_kit::test]
fn transport_exact_running_restart_retains_requested_until_exact_active_confirmation(
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
    let path = settings_test_path("exact-restart");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    open(cx);
    reveal(cx, "process-policy-1");
    cx.update(|window, cx| window.click("process-policy-1", cx));
    reveal(cx, "process-exact-open");
    cx.update(|window, cx| window.click("process-exact-open", cx));
    text(cx, "process-exact-target", "1344");
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    );
    text(cx, "process-exact-target", "not an integer");
    view.update(cx, |shell, cx| {
        shell.save_audio_settings(super::super::settings::Editor::Global, true, cx);
        assert!(shell.settings.editor.global.error.is_some());
        assert!(shell.settings.editor.global.pending.is_none());
    });
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AppRequest::ApplyGraph))
    );
    reveal(cx, "process-exact-target");
    cx.update(|window, cx| {
        window.click("process-exact-target", cx);
        window.press("ctrl-a", cx);
        window.input("1456", cx);
        view.update(cx, |shell, cx| {
            shell.save_audio_settings(super::super::settings::Editor::Global, true, cx)
        });
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
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(
            snapshot.transport_latency.process.unwrap().target_frames,
            1456
        );
        assert!(snapshot.active_transport_latency.unwrap().process.is_none());
        assert!(shell.settings.editor.global.pending.is_some());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .process
            .unwrap()
            .target_frames,
        1344
    );
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
                .process
                .unwrap()
                .target_frames,
            1456
        );
        assert!(shell.settings.editor.global.pending.is_none());
        assert!(!shell.settings.editor.global.dirty());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .process
            .unwrap()
            .target_frames,
        1456
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn transport_exact_vad_nonpreset_save_reads_current_text_without_a_render_frame(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("exact-vad-stopped");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    open(cx);
    reveal(cx, "vad-policy-1");
    cx.update(|window, cx| window.click("vad-policy-1", cx));
    reveal(cx, "vad-exact-open");
    cx.update(|window, cx| window.click("vad-exact-open", cx));
    text(cx, "vad-exact-capacity", "4800");
    reveal(cx, "vad-exact-target");
    cx.update(|window, cx| {
        window.click("vad-exact-target", cx);
        view.update(cx, |shell, cx| {
            assert!(shell.typing_focus(&window.focused(cx).unwrap(), cx));
        });
        window.press("ctrl-a", cx);
        window.input("945", cx);
        // Deliberately bypass a render/event-drain before transaction activation.
        view.update(cx, |shell, cx| {
            shell.save_audio_settings(super::super::settings::Editor::Global, false, cx)
        });
    });
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.phase, crate::service::SessionPhase::Idle);
        assert!(snapshot.active_transport_latency.is_none());
        assert!(snapshot.transport_latency.process.is_none());
        let vad = snapshot.transport_latency.vad.unwrap();
        assert_eq!((vad.capacity_frames, vad.target_frames), (4800, 945));
        assert_eq!(
            (vad.jitter_frames, vad.max_packet_frames, vad.max_gap_frames),
            (96, 480, 480)
        );
        assert_eq!(vad.provenance, crate::service::DeliveryProvenance::Declared);
        assert!(!shell.settings.editor.global.dirty());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .vad
            .unwrap()
            .target_frames,
        945
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn transport_exact_all_invalid_fields_and_relationships_refuse_previous_valid_value(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("exact-process-invalid");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    open(cx);
    reveal(cx, "process-policy-1");
    cx.update(|window, cx| window.click("process-policy-1", cx));
    reveal(cx, "process-exact-open");
    cx.update(|window, cx| window.click("process-exact-open", cx));
    for (field, invalid, valid) in [
        ("capacity", "262145", "16384"),
        ("target", "1344.5", "1344"),
        ("jitter", "-1", "96"),
        ("packet", "0", "480"),
        ("gap", "", "531"),
        ("target", "600", "1344"), // Valid integer, invalid combined gap+jitter.
    ] {
        let id = format!("process-exact-{field}");
        text(cx, &id, invalid);
        view.update(cx, |shell, cx| {
            shell.save_audio_settings(super::super::settings::Editor::Global, false, cx);
            assert!(
                shell.settings.editor.global.error.is_some(),
                "{field}: {invalid}"
            );
            assert!(shell.settings.editor.global.pending.is_none());
            assert!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .transport_latency
                    .process
                    .is_none()
            );
        });
        assert!(!path.exists(), "must not save old valid {field}");
        text(cx, &id, valid);
    }
    view.update(cx, |shell, cx| {
        shell.save_audio_settings(super::super::settings::Editor::Global, false, cx)
    });
    settle_settings(&view, cx);
    let process = crate::project::load_preferences_at(&path)
        .unwrap()
        .transport_latency
        .process
        .unwrap();
    assert_eq!(
        (
            process.capacity_frames,
            process.target_frames,
            process.jitter_frames,
            process.max_packet_frames,
            process.max_gap_frames
        ),
        (16384, 1344, 96, 480, 531)
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn transport_exact_policy_reset_and_discard_drop_invalid_hidden_fields(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("exact-reset");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    open(cx);
    reveal(cx, "vad-policy-1");
    cx.update(|window, cx| window.click("vad-policy-1", cx));
    reveal(cx, "vad-exact-open");
    cx.update(|window, cx| window.click("vad-exact-open", cx));
    text(cx, "vad-exact-target", "invalid");
    cx.update(|window, cx| {
        // Policy is above the exact inputs; scroll back before clicking.
        window.scroll(
            "settings-scroll",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(1800.))),
            cx,
        );
    });
    reveal(cx, "vad-policy-0");
    cx.update(|window, cx| {
        window.click("vad-policy-0", cx);
        window.click("vad-policy-1", cx);
    });
    reveal(cx, "vad-exact-open");
    cx.update(|window, cx| window.click("vad-exact-open", cx));
    reveal(cx, "vad-exact-target");
    cx.update(|window, _| assert_eq!(window.find("vad-exact-target").value(), Some("2048")));
    text(cx, "vad-exact-target", "bad again");
    cx.update(|window, cx| window.click("settings-discard", cx));
    view.update(cx, |shell, cx| {
        assert!(shell.settings.editor.global.transport.vad.is_none());
        shell.save_audio_settings(super::super::settings::Editor::Global, false, cx);
        assert!(shell.settings.editor.global.error.is_none());
    });
    settle_settings(&view, cx);
    assert!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .transport_latency
            .vad
            .is_none()
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
