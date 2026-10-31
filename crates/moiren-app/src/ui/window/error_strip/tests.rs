//! Error-strip cache tests (design 17 hazard 6): rebuild counters pair with
//! paint-output checks, never with the root's per-frame render count.

use std::sync::Arc;

use super::StudioShell;
use crate::service::{GraphFault, GraphSubject};
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};

/// One cached-path frame: notified views re-render, unnotified cached
/// subtrees replay their paint. `render_frame` refreshes the whole window and
/// would rebuild every panel regardless of the caches.
fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn strip_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| shell.error_strip.read(cx).rebuild_count())
}

/// Delivers a snapshot the way the service does — a fresh Arc behind a shell
/// notification — without a full-window `render_frame`, which rebuilds every
/// panel regardless of the caches.
fn publish_snapshot(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut crate::service::AppSnapshot),
) {
    view.update(cx, |shell, cx| {
        let mut snapshot = (*shell.state.read(cx).snapshot).clone();
        change(&mut snapshot);
        shell
            .state
            .update(cx, |state, _| state.snapshot = Arc::new(snapshot));
        cx.notify();
    });
    cached_frame(cx);
}

/// The strip mounts with the first faulty publish, stays cached across
/// key-neutral notifications, and repaints a second publish exactly once.
#[gpui_kit::test]
fn error_strip_mounts_with_a_publish_and_repaints_the_next_exactly_once(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    assert!(
        cx.update(|window, _| window.try_find("error-strip").is_none()),
        "an idle shell must not mount the strip"
    );

    publish_snapshot(&view, cx, |snapshot| {
        snapshot.error = Some("Probe publish failure".into());
    });
    assert_eq!(
        strip_rebuilds(&view, cx),
        1,
        "the mounting publish must render the strip exactly once"
    );
    assert!(cx.update(|window, _| window.find("error-strip-message").visible()));
    let message = cx.update(|window, _| window.find("error-strip-message").bounds());

    // A graph selection is shell state the strip never renders.
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        strip_rebuilds(&view, cx),
        1,
        "a key-neutral notification must replay the cached strip"
    );
    assert_eq!(
        cx.update(|window, _| window.find("error-strip-message").bounds()),
        message,
        "the replayed paint must not move"
    );

    publish_snapshot(&view, cx, |snapshot| {
        Arc::make_mut(&mut snapshot.graph).faults.push(GraphFault {
            subject: GraphSubject::Graph,
            cause: "Probe fault cause".into(),
            hint: None,
        });
    });
    assert_eq!(
        strip_rebuilds(&view, cx),
        2,
        "the second publish must rebuild the strip exactly once"
    );
    assert!(cx.update(|window, _| window.find("fault-0").visible()));
    finish_service(runtime);
}

/// A dispatch failure writes `state.dispatch_error` synchronously
/// (`UiState::try_dispatch`) with no publish behind it, and the strip's
/// `build` renders exactly that field — so the key carries the field and the
/// mounted strip repaints the diagnostic on that notification alone.
#[gpui_kit::test]
fn error_strip_repaints_a_dispatch_failure_while_already_mounted(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);

    // Mount the strip with a graph fault only: no `error-strip-message` row.
    publish_snapshot(&view, cx, |snapshot| {
        Arc::make_mut(&mut snapshot.graph).faults.push(GraphFault {
            subject: GraphSubject::Graph,
            cause: "Probe fault cause".into(),
            hint: None,
        });
    });
    assert!(cx.update(|window, _| window.find("fault-0").visible()));
    assert!(
        cx.update(|window, _| window.try_find("error-strip-message").is_none()),
        "a fault without a diagnostic renders no message row"
    );

    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch_error = Some("Probe dispatch failure".into())
        });
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        strip_rebuilds(&view, cx),
        2,
        "the dispatch failure must rebuild the mounted strip exactly once"
    );
    assert!(
        cx.update(|window, _| window.find("error-strip-message").visible()),
        "the dispatch diagnostic must reach the mounted strip without waiting for a publish"
    );
    finish_service(runtime);
}
