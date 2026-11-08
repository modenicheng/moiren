//! Device period text edits use the real settings transaction without native IO.
use super::*;

fn reveal(cx: &mut gpui_kit::VisualTestContext, id: &str) {
    reveal_in(cx, "settings-scroll", id);
}

fn reveal_in(cx: &mut gpui_kit::VisualTestContext, scroll: &str, id: &str) {
    cx.update(|window, cx| {
        for _ in 0..20 {
            let target = window.find(id.to_owned());
            let viewport = window.find(scroll.to_owned()).bounds();
            if target.visible() && viewport.contains(&target.bounds().center()) {
                return;
            }
            window.scroll(
                scroll.to_owned(),
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-150.))),
                cx,
            );
        }
        panic!("period control is not visible");
    });
}

#[gpui_kit::test]
fn shared_native_period_text_save_preserves_exact_request_without_starting_audio(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("shared-period");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.activate_window();
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    reveal(cx, "default-output-period-input");
    cx.update(|window, cx| {
        assert!(window.find("default-output-period-input").visible());
        window.click("default-output-period-input", cx);
        assert_eq!(
            window.find("default-output-period-input").focused(),
            Some(true)
        );
        // Native typing must traverse the modal before saving the request.
        window.input("336", cx);
        assert_eq!(
            window.find("default-output-period-input").value(),
            Some("336")
        );
        window.press("enter", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, _| {
        assert_eq!(
            shell.settings.editor.global.audio.output.buffer_frames,
            Some(336)
        );
        assert_eq!(
            shell.settings.editor.global.audio.processing.block_frames,
            256
        );
    });
    cx.update(|window, cx| window.click("settings-save", cx));
    settle_settings(&view, cx);
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.phase, crate::service::SessionPhase::Idle);
        assert_eq!(snapshot.audio_settings.output.buffer_frames, Some(336));
        assert!(snapshot.active_audio_settings.is_none());
        assert!(!shell.settings.editor.global.dirty());
    });
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .output
            .buffer_frames,
        Some(336)
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn shared_native_period_invalid_text_blocks_save_and_discard_restores_default(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("invalid-shared-period");
    view.update(cx, |shell, _| shell.preferences_path = Some(path.clone()));
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1440.), px(1000.)));
        window.render_frame(cx);
        window.click("open-settings", cx);
    });
    reveal(cx, "default-output-period-input");
    cx.update(|window, cx| {
        window.click("default-output-period-input", cx);
        window.input("336", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.press("ctrl-a", cx);
        window.input("4097", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.click("settings-save", cx);
    });
    cx.run_until_parked();
    view.update(cx, |shell, cx| {
        assert!(shell.settings.editor.global.error.is_some());
        assert!(shell.settings.editor.global.pending.is_none());
        assert_eq!(
            shell.settings.editor.global.audio.output.buffer_frames,
            Some(336)
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .audio_settings
                .output
                .buffer_frames,
            None
        );
    });
    assert!(
        !path.exists(),
        "invalid text must not persist the preceding valid value"
    );
    cx.update(|window, cx| window.click("settings-discard", cx));
    reveal(cx, "default-output-period-input");
    cx.update(|window, cx| {
        assert_eq!(window.find("default-output-period-input").value(), Some(""));
        window.click("default-output-period-input", cx);
        window.input("480", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.click("settings-save", cx);
    });
    settle_settings(&view, cx);
    reveal(cx, "default-output-period-input");
    cx.update(|window, cx| {
        window.click("default-output-period-input", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.click("settings-save", cx);
    });
    settle_settings(&view, cx);
    assert_eq!(
        crate::project::load_preferences_at(&path)
            .unwrap()
            .audio
            .output
            .buffer_frames,
        None
    );
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui_kit::test]
fn shared_native_period_device_input_keeps_focus_and_reset_drops_invalid_text(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let path = settings_test_path("device-shared-period");
    view.update(cx, |shell, cx| {
        shell.preferences_path = Some(path.clone());
        shell.set_page(pages::Page::Devices, cx);
        shell.inspect_device("output-a".into(), cx);
    });
    cx.update(|window, cx| {
        window.resize(gpui_kit::size(px(1600.), px(1200.)));
        window.render_frame(cx);
        window.click("device-override", cx);
    });
    reveal_in(cx, "device-override-scroll", "device-output-period-input");
    cx.update(|window, cx| {
        window.click("device-output-period-input", cx);
        window.input("336", cx);
        view.update(cx, |shell, cx| {
            assert!(shell.typing_focus(&window.focused(cx).unwrap(), cx));
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.click("device-settings-save", cx);
    });
    settle_settings(&view, cx);
    let saved = crate::project::load_preferences_at(&path).unwrap();
    assert_eq!(
        saved.audio.output_overrides["output-a"].buffer_frames,
        Some(336)
    );
    assert_eq!(saved.audio.output.buffer_frames, None);
    reveal_in(cx, "device-override-scroll", "device-output-period-input");
    cx.update(|window, cx| {
        window.click("device-output-period-input", cx);
        window.press("ctrl-a", cx);
        window.input("invalid", cx);
        window.click("device-use-defaults", cx);
        window.click("device-override", cx);
    });
    reveal_in(cx, "device-override-scroll", "device-output-period-input");
    cx.update(|window, cx| {
        assert_eq!(window.find("device-output-period-input").value(), Some(""));
        window.click("device-settings-save", cx);
    });
    settle_settings(&view, cx);
    let saved = crate::project::load_preferences_at(&path).unwrap();
    assert_eq!(saved.audio.output_overrides["output-a"].buffer_frames, None);
    finish_service(runtime);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
