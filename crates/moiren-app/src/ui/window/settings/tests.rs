//! Settings overlay cache tests (design 17 hazard 6): rebuild counters pair
//! with paint-output checks, never with the root's per-frame render count.

use super::{Editor, Section, StudioShell};
use crate::ui::widgets::actions::Density;
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};
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

/// Opens the settings overlay on a settled shell (no springs still moving).
fn open_settings(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
    });
    settle_panels(cx);
}

/// A graph selection is shell state the settings overlay never renders: the
/// edit plus its notifications must leave the overlay on its cached paint,
/// and the replayed paint must hold across further key-neutral notifications.
#[gpui_kit::test]
fn settings_panel_stays_cached_across_key_neutral_shell_notifications(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    open_settings(cx);
    let dialog = cx.update(|window, _| window.find("settings-dialog").bounds());
    let before = settings_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before,
        "a selection edit must not reach the settings overlay"
    );
    assert_eq!(
        cx.update(|window, _| window.find("settings-dialog").bounds()),
        dialog,
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

/// The section and the drafts are key inputs: each must repaint the overlay
/// exactly once per change, while a notification that moves neither replays
/// the cached paint.
#[gpui_kit::test]
fn settings_panel_repaints_section_and_draft_changes_exactly_once(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    open_settings(cx);
    assert!(
        cx.update(|window, _| window.try_find("density-0").is_none()),
        "the audio section renders no density choices"
    );
    let before = settings_rebuilds(&view, cx);

    // Mirrors the section tab click: `shell.settings.section = …` plus a
    // plain shell notification (the click path itself re-renders the whole
    // window, which would rebuild every panel regardless of the caches).
    view.update(cx, |shell, cx| {
        shell.settings.section = Section::Appearance;
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "the section switch must rebuild the overlay exactly once"
    );
    let density = cx.update(|window, _| window.find("density-0").bounds());
    assert!(
        density.size.width > gpui_kit::px(0.),
        "the appearance section must paint its density choices"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 1,
        "a key-neutral notification must replay the cached paint"
    );

    // A draft edit is a key input of its own, and the compact draft visibly
    // re-lays the section out (tighter scroll padding, denser controls).
    view.update(cx, |shell, cx| {
        shell.settings.draft_mut(Editor::Global).density = Density::Compact;
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        settings_rebuilds(&view, cx),
        before + 2,
        "the draft change must rebuild the overlay exactly once"
    );
    let compact = cx.update(|window, _| window.find("density-0").bounds());
    assert_ne!(
        compact.origin, density.origin,
        "the compact draft must repaint the section layout"
    );
    finish_service(runtime);
}
