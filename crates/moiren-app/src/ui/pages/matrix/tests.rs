//! Matrix page scroll tests. The page renders uncached (the scroll offset is
//! no longer a cache key: a scroll-coupled cache rebuilt every cell per wheel
//! step and let the frozen headers overshoot the clamp edge while the grid
//! never followed), so these pin the header/body scroll contract instead:
//! headers sit exactly on the offset the body applies — mid-range and at the
//! clamp edges — and density changes still land on a plain shell
//! notification.

use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Entity, ScrollDelta, SharedString, TestAppContext, VisualTestContext, point, px, size,
};

use super::{Page, StudioShell};
use crate::service::{AppRequest, GraphEdit};
use crate::ui::shell::test_harness::{finish_service, settle_panels, test_shell, wait_until};
use crate::ui::widgets::actions::Density;

/// One frame: notified views re-render and the scroll containers re-clamp.
fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

/// Drives the state's mailbox until the dispatched request publishes, like
/// the mixer suite's helper: the dispatch rides the mailbox and only a poll
/// consumes it.
fn poll(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    wait_until(Duration::from_secs(2), || {
        let pending = view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).pending_request
        });
        cx.update(|window, cx| window.render_frame(cx));
        !pending
    });
}

fn route_cell_height(cx: &mut VisualTestContext) -> f32 {
    cx.update(|window, _| f32::from(window.find("route-0-0").bounds().size.height))
}

/// Header/body alignment: the frozen header strips must sit on the same
/// offset the grid body applies, header column 0 on body column 0 and header
/// row 0 on body row 0. Compared with sub-pixel tolerance: the body's offset
/// is pixel-snapped at prepaint, the headers' is not.
fn assert_headers_align_with_body(cx: &mut VisualTestContext) {
    let ((strip_x, cell_x), (strip_y, cell_y)) = cx.update(|window, _| {
        let column = window.find("matrix-column-header-strip").bounds();
        let row = window.find("matrix-row-header-strip").bounds();
        let cell = window.find("route-0-0").bounds();
        (
            (column.origin.x, cell.origin.x),
            (row.origin.y, cell.origin.y),
        )
    });
    assert!(
        (strip_x - cell_x).abs() < px(1.),
        "column header must track the body x offset ({strip_x} vs {cell_x})"
    );
    assert!(
        (strip_y - cell_y).abs() < px(1.),
        "row header must track the body y offset ({strip_y} vs {cell_y})"
    );
}

/// Opens the Matrix page on a settled shell. Density is loaded from the
/// developer's real preferences at shell construction, so the baseline is
/// pinned explicitly.
fn open_matrix_page(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Matrix;
        shell.inspector_open = false;
        shell.preferences.values.density = crate::project::UiDensity::Comfortable;
        shell.density = Density::Comfortable;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring notifies its way through the first
    // second; let it finish before measuring anything.
    settle_panels(cx);
}

/// Fills the grid until it overflows both axes: sixteen 2-channel gains on
/// top of the default session, each waited through the service's publish.
fn grow_grid(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    for _ in 0..16 {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: moiren_core::graph::NodeKind::Gain,
                    channels: 2,
                }));
            });
            cx.notify();
        });
        poll(view, cx);
    }
    wait_until(Duration::from_secs(3), || {
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).snapshot.graph.graph.nodes().len() == 20
        })
    });
    frame(cx);
}

/// The header/body scroll contract, mid-range and at the clamp edge. The old
/// page broke the clamp half of this: the headers read an unclamped offset at
/// render while the body clamps at prepaint, so an overscroll showed the
/// header overshoot then snap back while the grid never followed.
#[gpui_kit::test]
fn matrix_headers_track_the_grid_scroll_through_the_clamp(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    open_matrix_page(&view, cx);
    grow_grid(&view, cx);

    // Mid-range: exact pixel scrolls move header and body together. One axis
    // per event: the scroll handler only applies the dominant axis of a
    // diagonal delta.
    cx.update(|window, cx| {
        window.scroll(
            "matrix-grid",
            ScrollDelta::Pixels(point(px(-200.), px(0.))),
            cx,
        );
        window.scroll(
            "matrix-grid",
            ScrollDelta::Pixels(point(px(0.), px(-150.))),
            cx,
        );
    });
    frame(cx);
    assert_headers_align_with_body(cx);

    // Overscroll far past both edges: the body clamps at its prepaint, and
    // the headers must land on that same clamp — the last cell's edges meet
    // the grid viewport's edges.
    cx.update(|window, cx| {
        window.scroll(
            "matrix-grid",
            ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
            cx,
        );
        window.scroll(
            "matrix-grid",
            ScrollDelta::Pixels(point(px(-100_000.), px(0.))),
            cx,
        );
    });
    frame(cx);
    assert_headers_align_with_body(cx);

    let (outputs, inputs) = view.read_with(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let inputs = graph
            .nodes()
            .iter()
            .map(|node| node.inputs().len())
            .sum::<usize>();
        let outputs = graph
            .nodes()
            .iter()
            .map(|node| node.outputs().len())
            .sum::<usize>();
        (outputs, inputs)
    });
    cx.update(|window, _| {
        let grid = window.find("matrix-grid").bounds();
        let last = window
            .find(SharedString::from(format!(
                "route-{}-{}",
                outputs - 1,
                inputs - 1
            )))
            .bounds();
        assert_eq!(
            last.bottom(),
            grid.bottom(),
            "the clamped body must end at the viewport's bottom edge"
        );
        assert_eq!(
            last.right(),
            grid.right(),
            "the clamped body must end at the viewport's right edge"
        );
    });
    finish_service(runtime);
}

/// The settings transaction applies a new density with a plain shell
/// notification (settings/transaction.rs); an idle session has no publishes
/// to paper over a key that missed the input, so the row height must follow
/// on that notification alone and hold across density-neutral frames.
#[gpui_kit::test]
fn matrix_rows_follow_density_on_a_plain_notification(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    open_matrix_page(&view, cx);
    let comfortable = route_cell_height(cx);
    // Mirrors the settings transaction: preferences.density and the theme
    // density move together.
    view.update(cx, |shell, cx| {
        shell.preferences.values.density = crate::project::UiDensity::Compact;
        shell.density = Density::Compact;
        cx.notify();
    });
    frame(cx);
    let compact = route_cell_height(cx);
    assert!(
        compact < comfortable,
        "compact rows must repaint shorter ({compact} vs {comfortable})"
    );
    frame(cx);
    assert_eq!(
        route_cell_height(cx),
        compact,
        "a density-neutral frame must keep the compact rows"
    );
    finish_service(runtime);
}
