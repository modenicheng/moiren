//! Exercise cached paint with a frozen service projection. The convenience
//! click/typing helpers refresh the whole window and can hide a missing key.
use super::{Section, SettingsKey, SettingsPanel, StudioShell};
use crate::service::{AppSnapshot, ExpertInputBuffer, ServiceRuntime};
use crate::ui::window::panel::CachedPanel;
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Entity, InputEvent, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext, point, px, size,
};
use std::sync::Arc;

pub(super) fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

pub(super) fn cached_click(cx: &mut VisualTestContext, id: &'static str) {
    // Scrolling is part of the real input path, not a whole-window refresh.
    let scroll = cx.update(|window, _| {
        let center = window.find(id).bounds().center();
        let viewport = window.find("settings-scroll").bounds();
        (!viewport.contains(&center)).then_some((viewport.center(), viewport.center().y - center.y))
    });
    if let Some((position, delta)) = scroll {
        cx.simulate_mouse_move(position, None, Modifiers::default());
        cx.update(|window, cx| {
            window.dispatch_event(
                ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(px(0.), delta)),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        });
        cached_frame(cx);
    }
    let center = cx.update(|window, _| {
        let target = window.find(id);
        assert!(target.visible(), "{id} must be visible before dispatch");
        let center = target.bounds().center();
        assert!(
            window.find("settings-dialog").bounds().contains(&center),
            "{id} must be inside the dialog"
        );
        center
    });
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cached_frame(cx);
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cached_frame(cx);
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    cached_frame(cx);
}

fn key(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> SettingsKey {
    view.read_with(cx, |shell, cx| SettingsPanel.key(shell, cx))
}

pub(super) fn feedback(cx: &mut VisualTestContext, id: &'static str) -> String {
    cx.update(|window, _| {
        window
            .find(id)
            .label()
            .expect("rendered feedback")
            .to_owned()
    })
}

pub(super) fn rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| {
        shell.settings_panel.read(cx).rebuild_count()
    })
}

pub(super) fn assert_frozen(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    snapshot: &Arc<AppSnapshot>,
) {
    view.read_with(cx, |shell, cx| {
        assert!(
            Arc::ptr_eq(snapshot, &shell.state.read(cx).snapshot),
            "a service publish must not repair this cache regression"
        );
    });
}

fn frozen_shell(
    cx: &mut TestAppContext,
    expert: bool,
) -> (
    Entity<StudioShell>,
    &mut VisualTestContext,
    ServiceRuntime,
    Arc<AppSnapshot>,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(2400.)));
    view.update(cx, |shell, cx| {
        shell.settings_open = true;
        shell.settings.section = Section::Transport;
        shell.settings.global = shell.current_settings_draft(cx);
        if expert {
            shell.settings.global.transport.process = Some(ExpertInputBuffer::default());
        }
        cx.notify();
    });
    settle_panels(cx);
    let snapshot = view.read_with(cx, |shell, cx| shell.state.read(cx).snapshot.clone());
    (view, cx, runtime, snapshot)
}

#[gpui_kit::test]
fn cache_transport_click_reveals_render_fields_and_enables_save(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    cached_click(cx, "render-policy-1");
    view.read_with(cx, |shell, _| {
        assert!(shell.settings.global.transport.render.is_some());
    });
    assert!(
        cx.update(|window, _| window.try_find("render-target-0").is_some()),
        "the transport-only edit must reveal the explicit render controls"
    );
    assert!(feedback(cx, "settings-feedback").contains("Unsaved changes"));
    cached_click(cx, "settings-discard");
    view.read_with(cx, |shell, _| {
        assert_eq!(shell.settings.global.transport.render, None);
    });
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[gpui_kit::test]
fn cache_expert_policy_click_reveals_input_fields(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    cached_click(cx, "process-policy-1");
    view.read_with(cx, |shell, _| {
        assert!(shell.settings.global.transport.process.is_some());
    });
    assert!(
        cx.update(|window, _| window.try_find("process-expert-fields").is_some()),
        "the expert policy must reveal its input bounds without a publish"
    );
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[gpui_kit::test]
fn cache_expert_exact_click_replaces_presets_without_a_publish(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, true);
    let before = key(&view, cx);
    cached_click(cx, "process-exact-open");
    assert!(
        key(&view, cx) != before,
        "expert exact mode must be a cache key input"
    );
    assert!(
        cx.update(|window, _| window.try_find("process-exact-target").is_some()),
        "the exact-mode flag must repaint the cached expert editor"
    );
    cached_click(cx, "process-exact-close");
    assert!(cx.update(|window, _| window.try_find("process-target-0").is_some()));
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[gpui_kit::test]
fn cache_measured_exact_click_replaces_presets_without_a_publish(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    let before = key(&view, cx);
    cached_click(cx, "measured-exact-open");
    assert!(
        key(&view, cx) != before,
        "measured exact mode must be a cache key input"
    );
    assert!(
        cx.update(|window, _| window.try_find("measured-exact-target").is_some()),
        "the measured exact-mode flag must repaint the cached editor"
    );
    cached_click(cx, "measured-exact-close");
    assert!(cx.update(|window, _| window.try_find("measured-target-0").is_some()));
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[cfg(windows)]
#[gpui_kit::test]
fn cache_live_window_click_and_policy_repaint_without_a_publish(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    let before = key(&view, cx);
    cached_click(cx, "measured-live-window-open");
    assert!(
        key(&view, cx) != before,
        "live-window mode must be a cache key input"
    );
    assert!(
        cx.update(|window, _| window.try_find("measured-live-window-policy-0-0").is_some()),
        "the live-window editor must open without a service publish"
    );
    assert!(feedback(cx, "measured-live-window-feedback").contains("all five"));
    for id in [
        "measured-live-window-policy-0-0",
        "measured-live-window-policy-1-0",
        "measured-live-window-policy-2-0",
        "measured-live-window-policy-3-0",
        "measured-live-window-policy-4-0",
    ] {
        cached_click(cx, id);
    }
    assert!(!feedback(cx, "measured-live-window-feedback").contains("all five"));
    cached_click(cx, "measured-live-window-close");
    assert!(cx.update(|window, _| window.try_find("measured-process-apply").is_some()));
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[cfg(windows)]
#[gpui_kit::test]
fn cache_retained_window_open_and_policy_repaint_without_a_publish(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    let before = key(&view, cx);
    cached_click(cx, "process-window-open");
    assert!(
        key(&view, cx) != before,
        "retained-window mode must be a cache key input"
    );
    assert!(
        cx.update(|window, _| window.try_find("process-window-feedback").is_some()),
        "the retained-prefix editor must open without a service publish"
    );
    let before = rebuilds(&view, cx);
    cached_click(cx, "process-window-policy-0-0");
    assert!(rebuilds(&view, cx) > before);
    assert!(feedback(cx, "process-window-feedback").contains("Stop a collected Process session"));
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}

#[gpui_kit::test]
fn cache_measured_feedback_repaints_but_unchanged_polling_reuses_paint(cx: &mut TestAppContext) {
    let (view, cx, runtime, snapshot) = frozen_shell(cx, false);
    view.update(cx, |shell, cx| {
        shell.settings.measured.error = Some("local measured refusal".into());
        cx.notify();
    });
    cached_frame(cx);
    assert!(feedback(cx, "measured-process-feedback").contains("local measured refusal"));
    let before = rebuilds(&view, cx);
    for _ in 0..4 {
        view.update(cx, |shell, cx| {
            shell.finish_measured_process(cx);
            shell.selected_node = Some(snapshot.graph.graph.nodes()[0].id());
            cx.notify();
        });
        cached_frame(cx);
    }
    assert_eq!(
        rebuilds(&view, cx),
        before,
        "unchanged measured observation and unrelated selection must reuse paint"
    );
    assert_frozen(&view, cx, &snapshot);
    finish_service(runtime);
}
