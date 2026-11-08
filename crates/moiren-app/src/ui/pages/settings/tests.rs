//! Settings page cache tests (design 17 hazard 6): rebuild counters pair
//! with paint-output checks, never with the root's per-frame render count.

use super::{Editor, Section, StudioShell};
use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell};
use crate::ui::widgets::actions::Density;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, size};

/// One cached-path frame: notified views re-render, unnotified cached
/// subtrees replay their paint. `render_frame` refreshes the whole window and
/// would rebuild every panel regardless of the caches.
fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn settings_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| {
        shell.settings_panel.read(cx).rebuild_count()
    })
}

/// Opens the settings workspace on a settled shell (no springs still
/// moving).
fn open_settings(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
    });
    settle_panels(cx);
}

/// A graph selection is shell state the settings page never renders: the
/// edit plus its notifications must leave the page on its cached paint, and
/// the replayed paint must hold across further key-neutral notifications.
#[gpui_kit::test]
fn settings_panel_stays_cached_across_key_neutral_shell_notifications(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    open_settings(cx);
    let page = cx.update(|window, _| window.find("settings-page").bounds());
    let before = settings_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before,
        "a selection edit must not reach the settings page"
    );
    assert_eq!(
        cx.update(|window, _| window.find("settings-page").bounds()),
        page,
        "the cached paint must not move"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before,
        "a key-neutral notification must replay the cached paint"
    );
    finish_service(runtime);
}

/// The section and the workstation density are key inputs: each must repaint
/// the page exactly once per change, while a notification that moves neither
/// replays the cached paint.
#[gpui_kit::test]
fn settings_panel_repaints_section_and_density_changes_exactly_once(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    open_settings(cx);
    assert!(
        cx.update(|window, _| window.try_find("density-0").is_none()),
        "the audio section renders no density choices"
    );
    let before = settings_rebuilds(&view, cx);

    // Mirrors the nav item click: `shell.settings.section = …` plus a
    // plain shell notification (the click path itself re-renders the whole
    // window, which would rebuild every panel regardless of the caches).
    view.update(cx, |shell, cx| {
        shell.settings.section = Section::Interface;
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "the section switch must rebuild the page exactly once"
    );
    let density = cx.update(|window, _| window.find("density-0").bounds());
    assert!(
        density.size.width > gpui_kit::px(0.),
        "the interface section must paint its density choices"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "a key-neutral notification must replay the cached paint"
    );

    // A density change is a key input of its own, and compact visibly
    // re-lays the section out (denser controls).
    view.update(cx, |shell, cx| {
        shell.density = Density::Compact;
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 2,
        "the density change must rebuild the page exactly once"
    );
    let compact = cx.update(|window, _| window.find("density-0").bounds());
    assert_ne!(
        compact.origin, density.origin,
        "the compact density must repaint the section layout"
    );
    finish_service(runtime);
}

/// The audio draft is a key input; the device draft is not — the settings
/// page never renders the device editor (it lives on the Devices page).
#[gpui_kit::test]
fn settings_page_rebuilds_on_global_draft_but_not_device_draft(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    open_settings(cx);
    let before = settings_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        use moiren_engine::audio_config::SAMPLE_RATES;
        shell
            .settings
            .draft_mut(Editor::Global)
            .audio
            .processing
            .sample_rate = SAMPLE_RATES[2];
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "a global draft edit must rebuild the page exactly once"
    );

    view.update(cx, |shell, cx| {
        use moiren_engine::audio_config::SAMPLE_RATES;
        shell
            .settings
            .draft_mut(Editor::Device)
            .audio
            .processing
            .sample_rate = SAMPLE_RATES[1];
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "a device draft edit must not reach the settings page"
    );
    finish_service(runtime);
}
