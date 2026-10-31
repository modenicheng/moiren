//! Matrix page cache tests (design 17 hazard 6): rebuild counters pair with
//! paint-output checks, never with the root's per-frame render count.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};

use super::{Page, StudioShell};
use crate::ui::widgets::actions::Density;
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};

/// One cached-path frame: notified views re-render, unnotified cached
/// subtrees replay their paint. `render_frame` refreshes the whole window and
/// would rebuild every panel regardless of the caches.
fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn matrix_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| shell.matrix_panel.read(cx).rebuild_count())
}

fn route_cell_height(cx: &mut VisualTestContext) -> f32 {
    cx.update(|window, _| f32::from(window.find("route-0-0").bounds().size.height))
}

/// Opens the Matrix page on a settled shell. Density is loaded from the
/// developer's real preferences at shell construction, so the baseline is
/// pinned explicitly.
fn open_matrix_page(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Matrix;
        shell.inspector_open = false;
        shell.preferences.density = crate::project::UiDensity::Comfortable;
        shell.density = Density::Comfortable;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring notifies its way through the first
    // second; let it finish or every frame rebuilds every panel.
    settle_panels(cx);
}

/// The panel with no nested models (HC-3): its cache must ride on the key
/// alone, so the shell notifies that a poll/publish emits — none of whose
/// inputs moved — leave the panel untouched, while a real key input moving
/// rebuilds exactly once.
#[gpui_kit::test]
fn matrix_panel_stays_cached_across_key_neutral_shell_notifications(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    open_matrix_page(&view, cx);
    let before = matrix_rebuilds(&view, cx);
    let grid = cx.update(|window, _| window.find("matrix-grid").bounds());
    for _ in 0..2 {
        view.update(cx, |_, cx| cx.notify());
        cached_frame(cx);
    }
    assert_eq!(
        matrix_rebuilds(&view, cx),
        before,
        "a shell notification whose key inputs all hold must not reach the panel"
    );
    assert_eq!(
        cx.update(|window, _| window.find("matrix-grid").bounds()),
        grid,
        "the cached paint must not move"
    );

    // Moving a key input rebuilds exactly once; the following key-neutral
    // notification replays the cached paint again.
    let route = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let from = graph
            .nodes()
            .iter()
            .find(|node| !node.outputs().is_empty())
            .unwrap();
        let source = from.outputs()[0].id();
        let destination = graph
            .nodes()
            .iter()
            .find(|node| !node.inputs().is_empty() && node.id() != from.id())
            .unwrap()
            .inputs()[0]
            .id();
        (source, destination)
    });
    view.update(cx, |shell, cx| {
        shell.selected_route = Some(route);
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        matrix_rebuilds(&view, cx),
        before + 1,
        "a key change must rebuild exactly once"
    );
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        matrix_rebuilds(&view, cx),
        before + 1,
        "the replayed paint must not rebuild the panel"
    );
    finish_service(runtime);
}

/// The settings transaction applies a new density with a plain shell
/// notification (settings/transaction.rs); an idle session has no publishes
/// to paper over a key that missed the input, so the row height must follow
/// on that notification alone and hold across density-neutral ones.
#[gpui_kit::test]
fn matrix_panel_rebuilds_rows_when_the_session_density_changes(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    open_matrix_page(&view, cx);
    let comfortable = route_cell_height(cx);
    let before = matrix_rebuilds(&view, cx);
    // Mirrors the settings transaction: preferences.density and the theme
    // density move together.
    view.update(cx, |shell, cx| {
        shell.preferences.density = crate::project::UiDensity::Compact;
        shell.density = Density::Compact;
        cx.notify();
    });
    cached_frame(cx);
    let compact = route_cell_height(cx);
    assert!(
        compact < comfortable,
        "compact rows must repaint shorter ({compact} vs {comfortable})"
    );
    assert_eq!(
        matrix_rebuilds(&view, cx),
        before + 1,
        "the density change must rebuild the panel exactly once"
    );
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(matrix_rebuilds(&view, cx), before + 1);
    assert_eq!(
        route_cell_height(cx),
        compact,
        "a density-neutral notification must keep the compact rows"
    );
    finish_service(runtime);
}
